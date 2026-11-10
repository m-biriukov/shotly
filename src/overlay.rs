//! One capture session: fullscreen overlay windows (one per display) showing
//! the frozen screen. Translates winit events into `Editor` calls and draws
//! its state. Ends with an `Outcome` the app turns into a toast.

use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::Result;
use winit::dpi::{LogicalPosition, LogicalSize, PhysicalPosition};
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{CursorIcon, Window, WindowId};

use crate::capture::{self, Shot};
use crate::config::Config;
use crate::editor::{Action, Cursor, EditKey, Editor};
use crate::geom::{Handle, Point, Rect, Scale};
use crate::keys::latin_shortcut;
use crate::layers::Layers;
use crate::render::Backdrop;
use crate::style::{Tool, Width};
use crate::toolbar::Item;
use crate::{macos, output, snap};

#[cfg(feature = "dev-hooks")]
mod profile;

struct Win {
    window: Rc<Window>,
    layers: Layers,
    shot: usize,
    /// RGBA copy of the screen for painting shapes; made on first use.
    backdrop: Option<Backdrop>,
    editor: Editor,
    cursor: Point,
    /// Layers need syncing with the editor.
    dirty: bool,
    tip: Tip,
}

/// Tooltip timing for one window: it appears after the cursor rests on a
/// button, and stays hidden after a click until the cursor moves elsewhere.
struct Tip {
    item: Option<Item>,
    since: Instant,
    shown: bool,
    suppressed: bool,
}

impl Tip {
    fn new() -> Self {
        Self {
            item: None,
            since: Instant::now(),
            shown: false,
            suppressed: false,
        }
    }

    fn waiting(&self) -> bool {
        self.item.is_some() && !self.shown && !self.suppressed
    }
}

/// How long the cursor rests on a button before its tooltip appears.
const TIP_DELAY: Duration = Duration::from_millis(550);

impl Win {
    fn size(&self) -> (u32, u32) {
        let s = self.window.inner_size();
        (s.width.max(1), s.height.max(1))
    }
}

pub enum Outcome {
    Cancelled,
    Copied,
    Saved(PathBuf),
    Failed(String),
}

pub struct Finished {
    pub outcome: Outcome,
    /// Last colour/size used, to remember for next time.
    pub color: usize,
    pub width: Width,
    /// Frame (global points) of the display the capture was on.
    pub display: (i32, i32, u32, u32),
}

/// Max gap and distance (points) between clicks of a double-click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
const DOUBLE_CLICK_PT: f64 = 4.0;

pub struct Session {
    cfg: Config,
    shots: Vec<Shot>,
    wins: Vec<Win>,
    modifiers: ModifiersState,
    last_click: Option<(Instant, usize, Point)>,
}

impl Session {
    /// Freezes every display and covers it with an overlay window.
    pub fn start(el: &ActiveEventLoop, cfg: &Config) -> Result<Self> {
        // Order matters: list and capture before any of our windows are visible.
        // The window list and the capture are independent; overlap them. The
        // capture takes ~60 ms in ScreenCaptureKit, so the (hidden) overlay
        // windows are created while it runs.
        let t0 = Instant::now();
        let listing = std::thread::spawn(snap::list_windows);
        let capture = capture::begin()?;
        let mut windows = Vec::new();
        for (origin, size) in capture.frames() {
            let attrs = Window::default_attributes()
                .with_title("screenshoot")
                .with_decorations(false)
                .with_resizable(false)
                .with_visible(false)
                .with_position(LogicalPosition::new(origin.0, origin.1))
                .with_inner_size(LogicalSize::new(size.0, size.1));
            let window = Rc::new(el.create_window(attrs)?);
            macos::raise_above_everything(&window);
            window.set_cursor(CursorIcon::Crosshair);
            windows.push(window);
        }
        let t1 = Instant::now();
        let shots = capture.finish()?;
        let frames = listing.join().unwrap_or_default();
        let t2 = Instant::now();
        let mut wins = Vec::with_capacity(shots.len());
        for (i, (shot, window)) in shots.iter().zip(windows).enumerate() {
            let s = window.inner_size();
            let bounds = Rect::new(0, 0, s.width as i32, s.height as i32);
            let scale = Scale(window.scale_factor());
            let layers = Layers::attach(&window, &shot.cg, scale, bounds)
                .ok_or_else(|| anyhow::anyhow!("could not set up overlay layers"))?;
            let mut editor = Editor::new(bounds, scale);
            editor.set_defaults(cfg.color, cfg.width());
            editor.set_snaps(snap::to_display(&frames, shot.origin_pt, scale, bounds));
            wins.push(Win {
                window,
                layers,
                shot: i,
                backdrop: None,
                editor,
                cursor: Point::new(0, 0),
                dirty: true,
                tip: Tip::new(),
            });
        }
        let mut session = Self {
            cfg: cfg.clone(),
            shots,
            wins,
            modifiers: ModifiersState::empty(),
            last_click: None,
        };
        // Layers are filled before the windows appear, so there is no flash.
        session.flush();
        macos::activate_app();
        for w in &session.wins {
            w.window.set_visible(true);
            w.layers.animate_start();
        }
        if let Some(w) = session.wins.first() {
            w.window.focus_window();
        }
        #[cfg(feature = "dev-hooks")]
        if std::env::var_os("SCREENSHOOT_PROFILE").is_some() {
            eprintln!(
                "profile: capture {:?} (windows created during it: {:?}), layers {:?}, total {:?}",
                t2 - t0,
                t1 - t0,
                t2.elapsed(),
                t0.elapsed()
            );
            profile::run(&mut session);
        }
        #[cfg(not(feature = "dev-hooks"))]
        let _ = (t0, t1, t2);
        Ok(session)
    }

    pub fn owns(&self, id: WindowId) -> bool {
        self.wins.iter().any(|w| w.window.id() == id)
    }

    fn win_index(&self, id: WindowId) -> Option<usize> {
        self.wins.iter().position(|w| w.window.id() == id)
    }

    /// Pushes pending editor changes into the layers. Called once per batch
    /// of input events, so a burst of mouse moves costs one update.
    pub fn flush(&mut self) {
        let now = Instant::now();
        for w in &mut self.wins {
            let item = w.editor.hover_target().map(|t| t.item);
            let was_shown = w.tip.shown;
            if item != w.tip.item {
                w.tip = Tip {
                    item,
                    since: now,
                    shown: false,
                    suppressed: false,
                };
            }
            if w.tip.waiting() && now.duration_since(w.tip.since) >= TIP_DELAY {
                w.tip.shown = true;
            }
            w.dirty |= w.tip.shown != was_shown;
            if !std::mem::take(&mut w.dirty) {
                continue;
            }
            let ed = &w.editor;
            let needs_pixels = ed.active_shape().is_some() || ed.static_shapes().next().is_some();
            if needs_pixels && w.backdrop.is_none() {
                let (bw, bh) = w.size();
                w.backdrop = Some(Backdrop::new(self.shots[w.shot].image(), bw, bh));
            }
            w.layers.sync(&w.editor, w.backdrop.as_ref(), w.tip.shown);
        }
    }

    /// When a tooltip is due, so the event loop wakes up for it.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.wins
            .iter()
            .filter(|w| w.tip.waiting())
            .map(|w| w.tip.since + TIP_DELAY)
            .min()
    }

    fn update_cursor(&self, i: usize) {
        let win = &self.wins[i];
        let icon = match win.editor.cursor(win.cursor) {
            Cursor::Crosshair => CursorIcon::Crosshair,
            Cursor::Move => CursorIcon::Move,
            Cursor::Pointer => CursorIcon::Default,
            Cursor::Text => CursorIcon::Text,
            Cursor::Resize(Handle::N | Handle::S) => CursorIcon::NsResize,
            Cursor::Resize(Handle::E | Handle::W) => CursorIcon::EwResize,
            Cursor::Resize(Handle::NW | Handle::SE) => CursorIcon::NwseResize,
            Cursor::Resize(Handle::NE | Handle::SW) => CursorIcon::NeswResize,
        };
        win.window.set_cursor(icon);
    }

    /// The window holding the capture area, if any.
    fn active(&self) -> Option<usize> {
        self.wins.iter().position(|w| w.editor.area().is_some())
    }

    fn finished(&self, i: usize, outcome: Outcome) -> Finished {
        let ed = &self.wins[i].editor;
        let shot = &self.shots[self.wins[i].shot];
        Finished {
            outcome,
            color: ed.color(),
            width: ed.width(),
            display: (
                shot.origin_pt.0,
                shot.origin_pt.1,
                shot.size_pt.0,
                shot.size_pt.1,
            ),
        }
    }

    fn act(&mut self, i: usize, action: Action) -> Option<Finished> {
        if action == Action::Cancel {
            return Some(self.finished(i, Outcome::Cancelled));
        }
        let i = self.active()?;
        self.wins[i].editor.finish_edit();
        let win = &self.wins[i];
        let (w, h) = win.size();
        let area = win.editor.area()?;
        let img = output::export(
            self.shots[win.shot].image(),
            area,
            win.editor.shapes(),
            w,
            h,
        );
        // Hide before the (possibly slow) encode so the screen comes back at once.
        for w in &self.wins {
            w.window.set_visible(false);
        }
        let outcome = if action == Action::Copy {
            match output::copy_to_clipboard(&img) {
                Ok(()) => Outcome::Copied,
                Err(e) => Outcome::Failed(format!("{e:#}")),
            }
        } else {
            let name = self.cfg.file_name(chrono::Local::now());
            match output::save(&img, &self.cfg.save_dir(), &name, self.cfg.format) {
                Ok(p) => Outcome::Saved(p),
                Err(e) => Outcome::Failed(format!("{e:#}")),
            }
        };
        Some(self.finished(i, outcome))
    }

    fn key(&mut self, i: usize, event: &KeyEvent) -> Option<Finished> {
        let cmd = self.modifiers.super_key();
        let shift = self.modifiers.shift_key();
        let key = event.logical_key.as_ref();
        // Shortcut letter in any layout: Cyrillic "к" on the R key is still R.
        let letter = latin_shortcut(&event.logical_key, event.physical_key);
        let editing = self
            .active()
            .is_some_and(|a| self.wins[a].editor.is_editing());

        if let Key::Named(NamedKey::Escape) = key
            && let Some(a) = self.active()
            && self.wins[a].editor.close_flyout()
        {
            self.wins[a].dirty = true;
            return None;
        }
        let action = match key {
            Key::Named(NamedKey::Escape) if !editing => Some(Action::Cancel),
            Key::Named(NamedKey::Enter) if !editing => Some(Action::Copy),
            _ if cmd && letter == Some('c') => Some(Action::Copy),
            _ if cmd && letter == Some('s') => Some(Action::Save),
            _ => None,
        };
        if let Some(a) = action {
            return self.act(i, a);
        }
        let i = self.active()?;
        let ed = &mut self.wins[i].editor;

        if cmd && letter == Some('z') {
            if shift { ed.redo() } else { ed.undo() }
        } else if editing {
            let edit_key = match key {
                Key::Named(NamedKey::Escape) => Some(EditKey::Finish),
                Key::Named(NamedKey::Enter) => Some(EditKey::Newline),
                Key::Named(NamedKey::Backspace) => Some(EditKey::Backspace),
                Key::Named(NamedKey::Delete) => Some(EditKey::Delete),
                Key::Named(NamedKey::ArrowLeft) => Some(EditKey::Left),
                Key::Named(NamedKey::ArrowRight) => Some(EditKey::Right),
                Key::Named(NamedKey::Home | NamedKey::ArrowUp) => Some(EditKey::Home),
                Key::Named(NamedKey::End | NamedKey::ArrowDown) => Some(EditKey::End),
                _ => None,
            };
            match (edit_key, &event.text) {
                (Some(k), _) => ed.edit_key(k),
                (None, Some(t)) if !cmd => ed.insert_text(t),
                _ => return None,
            }
        } else {
            match key {
                Key::Named(NamedKey::Backspace | NamedKey::Delete) => ed.delete_selected(),
                _ if !cmd => ed.set_tool(Tool::from_key(letter?)?),
                _ => return None,
            }
        }
        self.wins[i].dirty = true;
        self.update_cursor(i);
        None
    }

    /// True if this press completes a double-click.
    fn is_double_click(&mut self, i: usize, p: Point) -> bool {
        let now = Instant::now();
        let max = DOUBLE_CLICK_PT * self.wins[i].window.scale_factor();
        let double = self.last_click.is_some_and(|(t, w, q)| {
            w == i
                && now.duration_since(t) <= DOUBLE_CLICK
                && ((p.x - q.x) as f64).abs() <= max
                && ((p.y - q.y) as f64).abs() <= max
        });
        // A third click starts a new pair rather than another double.
        self.last_click = if double { None } else { Some((now, i, p)) };
        double
    }

    /// Handles an event for one of this session's windows. Returns `Some`
    /// when the session is over; the caller then drops it (closing windows).
    pub fn window_event(&mut self, id: WindowId, event: WindowEvent) -> Option<Finished> {
        let i = self.win_index(id)?;
        let shift = self.modifiers.shift_key();
        match event {
            WindowEvent::CloseRequested => return Some(self.finished(i, Outcome::Cancelled)),
            WindowEvent::Resized(s) => {
                self.wins[i]
                    .editor
                    .set_bounds(Rect::new(0, 0, s.width as i32, s.height as i32));
                self.wins[i].dirty = true;
            }
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::CursorMoved { position, .. } => {
                let p = to_point(position);
                let win = &mut self.wins[i];
                win.cursor = p;
                if win.editor.motion(p, shift) {
                    win.dirty = true;
                }
                self.update_cursor(i);
            }
            WindowEvent::CursorLeft { .. } => {
                // Drop the snap and button highlights on displays the cursor left.
                let win = &mut self.wins[i];
                if win.editor.clear_hover() {
                    win.dirty = true;
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                let p = self.wins[i].cursor;
                match state {
                    ElementState::Pressed => {
                        // Only one display can hold the capture area at a time.
                        for (j, other) in self.wins.iter_mut().enumerate() {
                            if j != i && other.editor.area().is_some() {
                                other.editor.clear_area();
                                other.dirty = true;
                            }
                        }
                        // A click dismisses the tooltip until the cursor moves to another button.
                        self.wins[i].tip.shown = false;
                        self.wins[i].tip.suppressed = true;
                        let double = self.is_double_click(i, p);
                        if let Some(action) = self.wins[i].editor.press(p, double) {
                            return self.act(i, action);
                        }
                    }
                    ElementState::Released => self.wins[i].editor.release(p, shift),
                }
                self.wins[i].dirty = true;
                self.update_cursor(i);
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                return self.key(i, &event);
            }
            _ => {}
        }
        None
    }
}

fn to_point(p: PhysicalPosition<f64>) -> Point {
    Point::new(p.x.round() as i32, p.y.round() as i32)
}
