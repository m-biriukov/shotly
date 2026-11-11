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
use crate::config::{Config, Preset};
use crate::editor::{Action, Cursor, EditKey, Editor};
use crate::geom::{Handle, Point, Rect, Scale};
use crate::keys::latin_shortcut;
use crate::layers::Layers;
use crate::record::Target;
use crate::render::Backdrop;
use crate::style::{Tool, Width};
use crate::toolbar::Item;
use crate::{macos, output, snap};

/// What the capture session is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Pick an area, annotate it, copy or save an image.
    Screenshot,
    /// Pick an area to record as video.
    Record,
}

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
    /// The user picked an area to record (record mode).
    Record(Target),
    Failed(String),
}

pub struct Finished {
    pub outcome: Outcome,
    /// Last colour/size used, to remember for next time.
    pub color: usize,
    pub width: Width,
    /// Preset in use when the session ended, to remember for next time.
    pub preset: Preset,
    /// Frame (global points) of the display the capture was on.
    pub display: macos::Frame,
}

/// Max gap and distance (points) between clicks of a double-click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
const DOUBLE_CLICK_PT: f64 = 4.0;

pub struct Session {
    mode: Mode,
    /// Window frames on screen when the session started (global points).
    frames: Vec<snap::Frame>,
    cfg: Config,
    shots: Vec<Shot>,
    wins: Vec<Win>,
    modifiers: ModifiersState,
    last_click: Option<(Instant, usize, Point)>,
}

impl Session {
    /// Freezes every display and covers it with an overlay window.
    pub fn start(el: &ActiveEventLoop, cfg: &Config, mode: Mode) -> Result<Self> {
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
                .with_title("shotly")
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
            if mode == Mode::Record {
                editor.set_record_mode();
            }
            editor.set_preset(cfg.preset);
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
            mode,
            frames,
            cfg: cfg.clone(),
            shots,
            wins,
            modifiers: ModifiersState::empty(),
            last_click: None,
        };
        // Full screen: the display under the pointer is selected right away.
        if cfg.preset == Preset::Full {
            let i = session.display_under_pointer().unwrap_or(0);
            session.choose_preset(i, Preset::Full);
        }
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
        if std::env::var_os("SHOTLY_PROFILE").is_some() {
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

    /// Index of the window whose display holds the mouse pointer.
    fn display_under_pointer(&self) -> Option<usize> {
        let (mx, my) = macos::mouse_location()?;
        self.wins.iter().position(|w| {
            let shot = &self.shots[w.shot];
            let (x, y) = (f64::from(shot.origin_pt.0), f64::from(shot.origin_pt.1));
            let (sw, sh) = (f64::from(shot.size_pt.0), f64::from(shot.size_pt.1));
            mx >= x && mx < x + sw && my >= y && my < y + sh
        })
    }

    /// Switches display `i` to `preset` (a key press).
    fn choose_preset(&mut self, i: usize, preset: Preset) {
        self.choose_preset_others(i, preset);
        self.wins[i].editor.choose_preset(preset);
        self.wins[i].dirty = true;
    }

    /// Keeps the other displays in step with display `i`: same preset, and
    /// only one display holds the selected area.
    fn choose_preset_others(&mut self, i: usize, preset: Preset) {
        for (j, w) in self.wins.iter_mut().enumerate() {
            if j != i {
                w.editor.set_preset(preset);
                w.editor.clear_area();
                w.dirty = true;
            }
        }
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
            preset: ed.preset(),
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
        if action == Action::Record {
            // Return before any area was picked does nothing.
            self.wins[i].editor.area()?;
            return Some(self.start_recording(i));
        }
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

    /// The picked area as a recording target. If it is exactly a window
    /// (the user clicked one), the recording follows that window.
    fn start_recording(&mut self, i: usize) -> Finished {
        let win = &self.wins[i];
        let shot = &self.shots[win.shot];
        let area = win.editor.area().unwrap_or(Rect::new(0, 0, 1, 1));
        let s = win.window.scale_factor();
        let picked = snap::Frame {
            id: None,
            x: f64::from(shot.origin_pt.0) + f64::from(area.x) / s,
            y: f64::from(shot.origin_pt.1) + f64::from(area.y) / s,
            w: f64::from(area.w) / s,
            h: f64::from(area.h) / s,
        };
        let window = window_at(&self.frames, &picked);
        let target = Target {
            display_id: shot.display_id,
            area: picked,
            window,
        };
        for w in &self.wins {
            w.window.set_visible(false);
        }
        self.finished(i, Outcome::Record(target))
    }

    /// The F / W / C preset keys. Returns true if the key was used.
    fn preset_key(&mut self, i: usize, event: &KeyEvent) -> bool {
        if self.modifiers.super_key() || self.modifiers.control_key() {
            return false;
        }
        let Some(p) =
            latin_shortcut(&event.logical_key, event.physical_key).and_then(Preset::from_key)
        else {
            return false;
        };
        // Letters are drawing tools too (not F, W or C), but never while typing text.
        if self
            .active()
            .is_some_and(|a| self.wins[a].editor.is_editing())
        {
            return false;
        }
        self.choose_preset(i, p);
        self.update_cursor(i);
        true
    }

    fn key(&mut self, i: usize, event: &KeyEvent) -> Option<Finished> {
        if self.preset_key(i, event) {
            return None;
        }
        if self.mode == Mode::Record {
            // Only Return (start) and Esc (cancel); no drawing tools here.
            return match event.logical_key.as_ref() {
                Key::Named(NamedKey::Escape) => self.act(i, Action::Cancel),
                Key::Named(NamedKey::Enter) => self.act(i, Action::Record),
                _ => None,
            };
        }
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
                        let before = self.wins[i].editor.preset();
                        if let Some(action) = self.wins[i].editor.press(p, double) {
                            return self.act(i, action);
                        }
                        // A strip click on this display applies on the others too.
                        let after = self.wins[i].editor.preset();
                        if after != before {
                            self.choose_preset_others(i, after);
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

/// The window whose frame is `picked` (within a point), if any.
fn window_at(frames: &[snap::Frame], picked: &snap::Frame) -> Option<u32> {
    const TOLERANCE_PT: f64 = 1.5;
    frames
        .iter()
        .find(|f| {
            (f.x - picked.x).abs() <= TOLERANCE_PT
                && (f.y - picked.y).abs() <= TOLERANCE_PT
                && (f.w - picked.w).abs() <= TOLERANCE_PT
                && (f.h - picked.h).abs() <= TOLERANCE_PT
        })
        .and_then(|f| f.id)
}

fn to_point(p: PhysicalPosition<f64>) -> Point {
    Point::new(p.x.round() as i32, p.y.round() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(id: u32, x: f64, y: f64, w: f64, h: f64) -> snap::Frame {
        snap::Frame {
            id: Some(id),
            x,
            y,
            w,
            h,
        }
    }

    fn frame_none(x: f64, y: f64, w: f64, h: f64) -> snap::Frame {
        snap::Frame {
            id: None,
            x,
            y,
            w,
            h,
        }
    }

    #[test]
    fn an_area_that_is_exactly_a_window_follows_it() {
        let frames = [
            frame(7, 100.0, 50.0, 800.0, 600.0),
            frame(9, 0.0, 0.0, 300.0, 200.0),
        ];
        let exact = frame_none(100.4, 49.6, 800.0, 600.9);
        assert_eq!(window_at(&frames, &exact), Some(7));
        // Dragged by hand, or resized: a plain area.
        assert_eq!(
            window_at(&frames, &frame_none(110.0, 50.0, 800.0, 600.0)),
            None
        );
        // A frame without an id is not followable.
        assert_eq!(
            window_at(
                &[frame_none(1.0, 1.0, 5.0, 5.0)],
                &frame_none(1.0, 1.0, 5.0, 5.0)
            ),
            None
        );
    }
}
