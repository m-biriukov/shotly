//! Native `AppKit` Settings window. Edits a copy of the config; "Save" writes
//! it to disk and tells the app (which re-registers the hotkey etc.).

use std::cell::{Cell, OnceCell, RefCell};
use std::ptr::{NonNull, null_mut};
use std::str::FromStr;

use anyhow::{Context, Result};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSBackingStoreType, NSButton, NSControlStateValueOff, NSControlStateValueOn,
    NSEvent, NSEventMask, NSEventModifierFlags, NSFont, NSModalResponseOK, NSOpenPanel,
    NSPopUpButton, NSTabView, NSTabViewItem, NSTextField, NSView, NSWindow, NSWindowDelegate,
    NSWindowStyleMask,
};
use objc2_foundation::{
    NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, ns_string,
};
use winit::event_loop::EventLoopProxy;

use crate::app::AppEvent;
use crate::config::{
    Config, FRAME_RATES, Format, HighlightSize, ToastCorner, ToastOffset, ToastSize,
};
use crate::login;
use crate::style::{COLOR_NAMES, Width};

const SIZE_NAMES: [&str; 3] = ["Small", "Medium", "Large"];

/// Which shortcut field is waiting for a key press.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Listening {
    No,
    Capture,
    Record,
}

struct Ivars {
    proxy: EventLoopProxy<AppEvent>,
    base: Config,
    hotkey: RefCell<String>,
    record_hotkey: RefCell<String>,
    listening: Cell<Listening>,
    monitor: RefCell<Option<Retained<AnyObject>>>,
    window: OnceCell<Retained<NSWindow>>,
    hotkey_button: OnceCell<Retained<NSButton>>,
    folder: OnceCell<Retained<NSTextField>>,
    filename: OnceCell<Retained<NSTextField>>,
    format: OnceCell<Retained<NSPopUpButton>>,
    color: OnceCell<Retained<NSPopUpButton>>,
    size: OnceCell<Retained<NSPopUpButton>>,
    toast: OnceCell<Retained<NSButton>>,
    toast_position: OnceCell<Retained<NSPopUpButton>>,
    toast_size: OnceCell<Retained<NSPopUpButton>>,
    toast_offset: OnceCell<Retained<NSPopUpButton>>,
    login: OnceCell<Retained<NSButton>>,
    record_hotkey_button: OnceCell<Retained<NSButton>>,
    fps: OnceCell<Retained<NSPopUpButton>>,
    resolution: OnceCell<Retained<NSPopUpButton>>,
    mic: OnceCell<Retained<NSButton>>,
    countdown: OnceCell<Retained<NSButton>>,
    outline: OnceCell<Retained<NSButton>>,
    highlight_cursor: OnceCell<Retained<NSButton>>,
    highlight_color: OnceCell<Retained<NSPopUpButton>>,
    highlight_size: OnceCell<Retained<NSPopUpButton>>,
    show_clicks: OnceCell<Retained<NSButton>>,
    left_color: OnceCell<Retained<NSPopUpButton>>,
    right_color: OnceCell<Retained<NSPopUpButton>>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    struct ShotlySettings;

    unsafe impl NSObjectProtocol for ShotlySettings {}

    unsafe impl NSWindowDelegate for ShotlySettings {
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _n: &NSNotification) {
            // The window does not retain its delegate; detach before we go away.
            if let Some(w) = self.ivars().window.get() {
                w.setDelegate(None);
            }
            if let Some(m) = self.ivars().monitor.borrow_mut().take() {
                // SAFETY: `m` came from addLocalMonitorForEventsMatchingMask.
                unsafe { NSEvent::removeMonitor(&m) };
            }
            let _ = self.ivars().proxy.send_event(AppEvent::SettingsClosed);
        }
    }

    impl ShotlySettings {
        #[unsafe(method(recordHotkey:))]
        fn record_hotkey(&self, _sender: Option<&AnyObject>) {
            self.listen(Listening::Capture);
        }

        #[unsafe(method(recordRecordHotkey:))]
        fn record_record_hotkey(&self, _sender: Option<&AnyObject>) {
            self.listen(Listening::Record);
        }

        #[unsafe(method(chooseFolder:))]
        fn choose_folder(&self, _sender: Option<&AnyObject>) {
            let panel = NSOpenPanel::openPanel(self.mtm());
            panel.setCanChooseDirectories(true);
            panel.setCanChooseFiles(false);
            panel.setCanCreateDirectories(true);
            if panel.runModal() == NSModalResponseOK
                && let Some(path) = panel.URL().and_then(|u| u.path())
                && let Some(f) = self.ivars().folder.get()
            {
                f.setStringValue(&NSString::from_str(&abbreviate_home(&path.to_string())));
            }
        }

        #[unsafe(method(save:))]
        fn save(&self, _sender: Option<&AnyObject>) {
            let cfg = self.collect();
            if let Err(e) = cfg.save() {
                eprintln!("could not save settings: {e:#}");
                return;
            }
            let want_login =
                self.ivars().login.get().is_some_and(|b| b.state() == NSControlStateValueOn);
            if want_login != login::is_enabled()
                && let Err(e) = login::set_enabled(want_login)
            {
                eprintln!("launch at login: {e:#}");
            }
            let _ = self.ivars().proxy.send_event(AppEvent::ConfigChanged(cfg));
            if let Some(w) = self.ivars().window.get() {
                w.close();
            }
        }

        #[unsafe(method(previewToast:))]
        fn preview_toast(&self, _sender: Option<&AnyObject>) {
            // Shows what the notice looks like with the choices made so far.
            let _ = self.ivars().proxy.send_event(AppEvent::PreviewToast(self.collect()));
        }

        #[unsafe(method(cancel:))]
        fn cancel(&self, _sender: Option<&AnyObject>) {
            if let Some(w) = self.ivars().window.get() {
                w.close();
            }
        }
    }
);

impl ShotlySettings {
    fn collect(&self) -> Config {
        let iv = self.ivars();
        let mut cfg = iv.base.clone();
        cfg.hotkey = iv.hotkey.borrow().clone();
        if let Some(f) = iv.folder.get() {
            cfg.save_dir = f.stringValue().to_string();
        }
        if let Some(f) = iv.filename.get() {
            cfg.filename = f.stringValue().to_string();
        }
        if let Some(p) = iv.format.get() {
            cfg.format = if p.indexOfSelectedItem() == 1 {
                Format::Jpeg
            } else {
                Format::Png
            };
        }
        if let Some(p) = iv.color.get() {
            cfg.color = p.indexOfSelectedItem().clamp(0, 7) as usize;
        }
        if let Some(p) = iv.size.get() {
            cfg.set_width(Width::ALL[p.indexOfSelectedItem().clamp(0, 2) as usize]);
        }
        if let Some(b) = iv.toast.get() {
            cfg.toast = b.state() == NSControlStateValueOn;
        }
        let pick = |p: &Option<&Retained<NSPopUpButton>>, n: usize| {
            p.map_or(0, |p| {
                p.indexOfSelectedItem().clamp(0, n as isize - 1) as usize
            })
        };
        cfg.toast_position =
            ToastCorner::ALL[pick(&iv.toast_position.get(), ToastCorner::ALL.len())];
        cfg.toast_size = ToastSize::ALL[pick(&iv.toast_size.get(), ToastSize::ALL.len())];
        cfg.toast_offset = ToastOffset::ALL[pick(&iv.toast_offset.get(), ToastOffset::ALL.len())];

        cfg.record_hotkey = iv.record_hotkey.borrow().clone();
        cfg.record_fps = FRAME_RATES[pick(&iv.fps.get(), FRAME_RATES.len())];
        let checked =
            |b: Option<&Retained<NSButton>>| b.is_some_and(|b| b.state() == NSControlStateValueOn);
        cfg.record_retina = pick(&iv.resolution.get(), 2) == 0;
        cfg.record_mic = checked(iv.mic.get());
        cfg.record_countdown = checked(iv.countdown.get());
        cfg.record_outline = checked(iv.outline.get());
        cfg.highlight_cursor = checked(iv.highlight_cursor.get());
        cfg.show_clicks = checked(iv.show_clicks.get());
        let colors = crate::style::PALETTE.len();
        cfg.highlight_color = pick(&iv.highlight_color.get(), colors);
        cfg.left_click_color = pick(&iv.left_color.get(), colors);
        cfg.right_click_color = pick(&iv.right_color.get(), colors);
        cfg.highlight_size =
            HighlightSize::ALL[pick(&iv.highlight_size.get(), HighlightSize::ALL.len())];
        cfg
    }

    /// Starts waiting for a key press for one of the two shortcut fields.
    fn listen(&self, which: Listening) {
        let iv = self.ivars();
        self.stop_listening();
        iv.listening.set(which);
        let button = match which {
            Listening::Capture => iv.hotkey_button.get(),
            Listening::Record => iv.record_hotkey_button.get(),
            Listening::No => None,
        };
        if let Some(b) = button {
            b.setTitle(ns_string!("Type shortcut… (Esc to cancel)"));
        }
    }

    /// Key events while the Settings window is key. Returns true to swallow.
    fn handle_key(&self, ev: &NSEvent) -> bool {
        let iv = self.ivars();
        let ours = match (ev.window(self.mtm()), iv.window.get()) {
            (Some(a), Some(b)) => Retained::as_ptr(&a) == Retained::as_ptr(b),
            _ => false,
        };
        if !ours {
            return false;
        }
        let flags = ev.modifierFlags();
        let code = ev.keyCode();
        let listening = iv.listening.get();
        if listening != Listening::No {
            if code == 53 {
                // Esc: keep the old shortcut.
                self.stop_listening();
                return true;
            }
            if let Some(s) = shortcut_string(flags, code)
                && global_hotkey::hotkey::HotKey::from_str(&s).is_ok()
            {
                match listening {
                    Listening::Capture => *iv.hotkey.borrow_mut() = s,
                    Listening::Record => *iv.record_hotkey.borrow_mut() = s,
                    Listening::No => {}
                }
                self.stop_listening();
            }
            return true;
        }
        // Accessory apps have no Edit menu, so wire the usual text shortcuts by hand.
        if flags.contains(NSEventModifierFlags::Command) {
            let action = match code {
                8 => Some(sel!(copy:)),
                9 => Some(sel!(paste:)),
                7 => Some(sel!(cut:)),
                0 => Some(sel!(selectAll:)),
                6 if flags.contains(NSEventModifierFlags::Shift) => Some(sel!(redo:)),
                6 => Some(sel!(undo:)),
                13 => Some(sel!(performClose:)),
                _ => None,
            };
            if let Some(action) = action {
                let app = NSApplication::sharedApplication(self.mtm());
                // SAFETY: standard responder-chain actions with no target/sender.
                return unsafe { app.sendAction_to_from(action, None, None) };
            }
        }
        false
    }

    /// Stops waiting for a key and shows both shortcuts again.
    fn stop_listening(&self) {
        let iv = self.ivars();
        iv.listening.set(Listening::No);
        for (button, shortcut) in [
            (iv.hotkey_button.get(), &iv.hotkey),
            (iv.record_hotkey_button.get(), &iv.record_hotkey),
        ] {
            if let Some(b) = button {
                b.setTitle(&NSString::from_str(&pretty_shortcut(&shortcut.borrow())));
            }
        }
    }
}

/// Owns the Settings window while it is open.
pub struct Settings {
    ctrl: Retained<ShotlySettings>,
}

impl Settings {
    pub fn open(cfg: &Config, proxy: EventLoopProxy<AppEvent>) -> Result<Self> {
        let mtm = MainThreadMarker::new().context("Settings must open on the main thread")?;
        let ctrl = ShotlySettings::alloc(mtm).set_ivars(Ivars {
            proxy,
            base: cfg.clone(),
            hotkey: RefCell::new(cfg.hotkey.clone()),
            record_hotkey: RefCell::new(cfg.record_hotkey.clone()),
            listening: Cell::new(Listening::No),
            monitor: RefCell::new(None),
            window: OnceCell::new(),
            hotkey_button: OnceCell::new(),
            folder: OnceCell::new(),
            filename: OnceCell::new(),
            format: OnceCell::new(),
            color: OnceCell::new(),
            size: OnceCell::new(),
            toast: OnceCell::new(),
            toast_position: OnceCell::new(),
            toast_size: OnceCell::new(),
            toast_offset: OnceCell::new(),
            login: OnceCell::new(),
            record_hotkey_button: OnceCell::new(),
            fps: OnceCell::new(),
            resolution: OnceCell::new(),
            mic: OnceCell::new(),
            countdown: OnceCell::new(),
            outline: OnceCell::new(),
            highlight_cursor: OnceCell::new(),
            highlight_color: OnceCell::new(),
            highlight_size: OnceCell::new(),
            show_clicks: OnceCell::new(),
            left_color: OnceCell::new(),
            right_color: OnceCell::new(),
        });
        // SAFETY: plain NSObject init of our subclass.
        let ctrl: Retained<ShotlySettings> = unsafe { msg_send![super(ctrl), init] };
        build(&ctrl, cfg, mtm);

        let weak = objc2::rc::Weak::from_retained(&ctrl);
        let block = RcBlock::new(move |ev: NonNull<NSEvent>| -> *mut NSEvent {
            // SAFETY: AppKit passes a valid event.
            let ev_ref = unsafe { ev.as_ref() };
            match weak.load() {
                Some(c) if c.handle_key(ev_ref) => null_mut(),
                _ => ev.as_ptr(),
            }
        });
        // SAFETY: the handler returns the event or null, as required.
        let monitor = unsafe {
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &block)
        };
        *ctrl.ivars().monitor.borrow_mut() = monitor;

        let s = Self { ctrl };
        s.show();
        Ok(s)
    }

    pub fn show(&self) {
        crate::macos::activate_app();
        if let Some(w) = self.ctrl.ivars().window.get() {
            w.makeKeyAndOrderFront(None);
        }
    }

    /// Closes the window (the app then hears `SettingsClosed`).
    pub fn close(&self) {
        if let Some(w) = self.ctrl.ivars().window.get() {
            w.close();
        }
    }

    pub fn set_login_checked(&self, on: bool) {
        if let Some(b) = self.ctrl.ivars().login.get() {
            b.setState(if on {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
        }
    }
}

const W: f64 = 520.0;
/// Height of one tab's content, and of the tab strip plus margins around it.
const PAGE_H: f64 = 480.0;
const TABS_CHROME: f64 = 44.0;
const BUTTONS_H: f64 = 60.0;
const H: f64 = PAGE_H + TABS_CHROME + BUTTONS_H;
const LABEL_W: f64 = 150.0;
const PAD: f64 = 20.0;
/// Where controls start inside a tab, right of the labels.
const X: f64 = PAD + LABEL_W + 12.0;
const ROW_H: f64 = 34.0;

/// Lays out a tab's rows top to bottom. A label and its control share one
/// vertical centre, so their text lines up whatever each one's height is.
struct Page {
    view: Retained<NSView>,
    top: f64,
    mtm: MainThreadMarker,
}

impl Page {
    fn new(mtm: MainThreadMarker) -> Self {
        let view = NSView::initWithFrame(
            NSView::alloc(mtm),
            NSRect::new(NSPoint::ZERO, NSSize::new(W - 2.0 * PAD + 20.0, PAGE_H)),
        );
        Self {
            view,
            top: 14.0,
            mtm,
        }
    }

    /// Vertical centre (Cocoa y, pointing up) of a row `h` tall; moves on.
    fn next(&mut self, h: f64) -> f64 {
        let yc = PAGE_H - self.top - h / 2.0;
        self.top += h;
        yc
    }

    /// `label` (may be empty) and its `control`, `w` wide. The text of both
    /// sits on one baseline. Returns the row's vertical centre.
    fn row(&mut self, label: &str, control: &NSView, w: f64) -> f64 {
        let yc = self.next(ROW_H);
        place(&self.view, control, X, yc, w);
        if !label.is_empty() {
            let l = NSTextField::labelWithString(&NSString::from_str(label), self.mtm);
            l.setAlignment(objc2_app_kit::NSTextAlignment::Right);
            place(&self.view, &l, PAD, yc, LABEL_W);
            align_baseline(&l, baseline(control));
        }
        yc
    }

    /// A row with a second control `dx` to the right of the first.
    fn row_with(
        &mut self,
        label: &str,
        control: &NSView,
        w: f64,
        extra: &NSView,
        dx: f64,
        ew: f64,
    ) {
        let yc = self.row(label, control, w);
        place(&self.view, extra, X + dx, yc, ew);
        align_baseline(extra, baseline(control));
    }

    /// Small grey note under a row.
    fn hint(&mut self, text: &str) {
        let yc = self.next(20.0);
        let hint = NSTextField::labelWithString(&NSString::from_str(text), self.mtm);
        hint.setFont(Some(&NSFont::systemFontOfSize(11.0)));
        place(&self.view, &hint, X, yc, 310.0);
    }

    /// Extra space between groups of rows.
    fn gap(&mut self, h: f64) {
        self.top += h;
    }
}

fn build(ctrl: &Retained<ShotlySettings>, cfg: &Config, mtm: MainThreadMarker) {
    let iv = ctrl.ivars();
    let target: &AnyObject = ctrl.as_ref();
    let window = new_window(ctrl, mtm);
    let view = window.contentView().expect("content view");

    // ---- General ----
    let mut general = Page::new(mtm);
    let hk = button(
        &pretty_shortcut(&cfg.hotkey),
        target,
        sel!(recordHotkey:),
        mtm,
    );
    general.row("Capture shortcut:", &hk, 250.0);

    let folder = NSTextField::textFieldWithString(&NSString::from_str(&cfg.save_dir), mtm);
    let choose = button("Choose…", target, sel!(chooseFolder:), mtm);
    general.row_with("Save folder:", &folder, 210.0, &choose, 216.0, 96.0);

    let filename = NSTextField::textFieldWithString(&NSString::from_str(&cfg.filename), mtm);
    general.row("File name:", &filename, 310.0);
    general.hint("{date} and {time} are filled in");

    let format = popup(
        &["PNG", "JPEG (quality 90)"],
        usize::from(cfg.format == Format::Jpeg),
        mtm,
    );
    general.row("Format:", &format, 200.0);

    let color = popup(&COLOR_NAMES, cfg.color, mtm);
    general.row("Default color:", &color, 200.0);

    let size_idx = Width::ALL
        .iter()
        .position(|&w| w == cfg.width())
        .unwrap_or(1);
    let size = popup(&SIZE_NAMES, size_idx, mtm);
    general.row("Default size:", &size, 200.0);

    general.gap(10.0);
    let login_box = checkbox("Launch at login", login::is_enabled(), mtm);
    general.row("", &login_box, 310.0);

    // ---- Notice ----
    let mut notice = Page::new(mtm);
    let toast = checkbox("Show notice after copy or save", cfg.toast, mtm);
    notice.row("", &toast, 310.0);

    let names = ToastCorner::ALL.map(ToastCorner::name);
    let toast_position = popup(
        &names,
        index_of(&ToastCorner::ALL, &cfg.toast_position),
        mtm,
    );
    notice.row("Position:", &toast_position, 200.0);

    let names = ToastSize::ALL.map(ToastSize::name);
    let toast_size = popup(&names, index_of(&ToastSize::ALL, &cfg.toast_size), mtm);
    notice.row("Size:", &toast_size, 200.0);

    let names = ToastOffset::ALL.map(ToastOffset::name);
    let toast_offset = popup(&names, index_of(&ToastOffset::ALL, &cfg.toast_offset), mtm);
    let preview = button("Preview", target, sel!(previewToast:), mtm);
    notice.row_with(
        "Distance from edge:",
        &toast_offset,
        200.0,
        &preview,
        210.0,
        100.0,
    );

    // ---- Recording ----
    let mut rec = Page::new(mtm);
    let rec_hk = button(
        &pretty_shortcut(&cfg.record_hotkey),
        target,
        sel!(recordRecordHotkey:),
        mtm,
    );
    rec.row("Record shortcut:", &rec_hk, 250.0);
    rec.hint("Press it again to stop recording");

    let fps_names = FRAME_RATES.map(|f| format!("{f} fps"));
    let fps_names: Vec<&str> = fps_names.iter().map(String::as_str).collect();
    let fps = popup(&fps_names, index_of(&FRAME_RATES, &cfg.record_fps), mtm);
    rec.row("Frame rate:", &fps, 200.0);

    let resolution = popup(
        &["Retina (full pixels)", "Standard (points)"],
        usize::from(!cfg.record_retina),
        mtm,
    );
    rec.row("Resolution:", &resolution, 200.0);

    let mic = checkbox("Record microphone", cfg.record_mic, mtm);
    rec.row("", &mic, 310.0);
    let countdown = checkbox(
        "Count down 3-2-1 before starting",
        cfg.record_countdown,
        mtm,
    );
    rec.row("", &countdown, 310.0);
    let outline = checkbox("Show a frame around the area", cfg.record_outline, mtm);
    rec.row("", &outline, 310.0);

    rec.gap(10.0);
    let highlight_cursor = checkbox("Highlight the mouse pointer", cfg.highlight_cursor, mtm);
    rec.row("", &highlight_cursor, 310.0);
    let highlight_color = popup(&COLOR_NAMES, cfg.highlight_color, mtm);
    rec.row("Highlight color:", &highlight_color, 200.0);
    let names = HighlightSize::ALL.map(HighlightSize::name);
    let highlight_size = popup(
        &names,
        index_of(&HighlightSize::ALL, &cfg.highlight_size),
        mtm,
    );
    rec.row("Highlight size:", &highlight_size, 200.0);

    rec.gap(10.0);
    let show_clicks = checkbox("Show mouse clicks", cfg.show_clicks, mtm);
    rec.row("", &show_clicks, 310.0);
    let left_color = popup(&COLOR_NAMES, cfg.left_click_color, mtm);
    rec.row("Left click color:", &left_color, 200.0);
    let right_color = popup(&COLOR_NAMES, cfg.right_click_color, mtm);
    rec.row("Right click color:", &right_color, 200.0);

    // ---- Tabs and buttons ----
    let tabs = NSTabView::initWithFrame(
        NSTabView::alloc(mtm),
        NSRect::new(
            NSPoint::new(PAD - 10.0, BUTTONS_H),
            NSSize::new(W - 2.0 * (PAD - 10.0), PAGE_H + TABS_CHROME),
        ),
    );
    for (title, page) in [
        ("General", &general),
        ("Notice", &notice),
        ("Recording", &rec),
    ] {
        let item = NSTabViewItem::new();
        item.setLabel(&NSString::from_str(title));
        item.setView(Some(&page.view));
        tabs.addTabViewItem(&item);
    }
    view.addSubview(&tabs);

    let save = button("Save", target, sel!(save:), mtm);
    let cancel = button("Cancel", target, sel!(cancel:), mtm);
    save.setKeyEquivalent(ns_string!("\r"));
    cancel.setKeyEquivalent(ns_string!("\u{1b}"));
    place(&view, &save, W - 24.0 - 96.0, 30.0, 96.0);
    place(&view, &cancel, W - 24.0 - 96.0 - 8.0 - 96.0, 30.0, 96.0);

    let _ = iv.hotkey_button.set(hk);
    let _ = iv.folder.set(folder);
    let _ = iv.filename.set(filename);
    let _ = iv.format.set(format);
    let _ = iv.color.set(color);
    let _ = iv.size.set(size);
    let _ = iv.toast.set(toast);
    let _ = iv.toast_position.set(toast_position);
    let _ = iv.toast_size.set(toast_size);
    let _ = iv.toast_offset.set(toast_offset);
    let _ = iv.login.set(login_box);
    let _ = iv.record_hotkey_button.set(rec_hk);
    let _ = iv.fps.set(fps);
    let _ = iv.resolution.set(resolution);
    let _ = iv.mic.set(mic);
    let _ = iv.countdown.set(countdown);
    let _ = iv.outline.set(outline);
    let _ = iv.highlight_cursor.set(highlight_cursor);
    let _ = iv.highlight_color.set(highlight_color);
    let _ = iv.highlight_size.set(highlight_size);
    let _ = iv.show_clicks.set(show_clicks);
    let _ = iv.left_color.set(left_color);
    let _ = iv.right_color.set(right_color);
    window.center();
    let _ = iv.window.set(window);
}

/// The titled, closable Settings window, delegating to `ctrl`.
fn new_window(ctrl: &Retained<ShotlySettings>, mtm: MainThreadMarker) -> Retained<NSWindow> {
    // SAFETY: standard window creation; released-when-closed is turned off
    // because we hold a Retained reference.
    let window = unsafe {
        let w = NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(W, H)),
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
            NSBackingStoreType::Buffered,
            false,
        );
        w.setReleasedWhenClosed(false);
        w
    };
    window.setTitle(ns_string!("Shotly Settings"));
    window.setDelegate(Some(ProtocolObject::from_ref(&**ctrl)));
    window
}

/// Push button that sends `action` to `target` (our controller).
fn button(
    title: &str,
    target: &AnyObject,
    action: objc2::runtime::Sel,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    // SAFETY: `target` is the controller, which implements every action
    // selector used here and outlives the window's buttons.
    unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str(title),
            Some(target),
            Some(action),
            mtm,
        )
    }
}

fn checkbox(title: &str, on: bool, mtm: MainThreadMarker) -> Retained<NSButton> {
    // SAFETY: a checkbox with no target/action; its state is read on Save.
    let b = unsafe {
        NSButton::checkboxWithTitle_target_action(&NSString::from_str(title), None, None, mtm)
    };
    b.setState(if on {
        NSControlStateValueOn
    } else {
        NSControlStateValueOff
    });
    b
}

/// Position of `value` in `all` (0 if absent).
fn index_of<T: PartialEq>(all: &[T], value: &T) -> usize {
    all.iter().position(|v| v == value).unwrap_or(0)
}

fn popup(items: &[&str], selected: usize, mtm: MainThreadMarker) -> Retained<NSPopUpButton> {
    let p = NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), NSRect::ZERO, false);
    for item in items {
        p.addItemWithTitle(&NSString::from_str(item));
    }
    p.selectItemAtIndex(selected.min(items.len() - 1) as isize);
    p
}

/// Where the first line of text in `v` sits, as a y in its superview.
fn baseline(v: &NSView) -> f64 {
    let f = v.frame();
    f.origin.y + f.size.height - v.firstBaselineOffsetFromTop()
}

/// Moves `v` up or down so its first line of text sits at `baseline`.
fn align_baseline(v: &NSView, baseline: f64) {
    let mut f = v.frame();
    f.origin.y = (baseline - (f.size.height - v.firstBaselineOffsetFromTop())).round();
    v.setFrame(f);
}

/// Adds `v` to `view` with its left edge at `x`, `w` wide, its natural
/// height, and its vertical centre at `y_center`.
fn place(view: &NSView, v: &NSView, x: f64, y_center: f64, w: f64) {
    let h = v.fittingSize().height.max(20.0);
    v.setFrame(NSRect::new(
        NSPoint::new(x, (y_center - h / 2.0).round()),
        NSSize::new(w, h),
    ));
    view.addSubview(v);
}

fn abbreviate_home(path: &str) -> String {
    match dirs::home_dir().and_then(|h| {
        path.strip_prefix(h.to_string_lossy().as_ref())
            .map(str::to_string)
    }) {
        Some(rest) => format!("~{rest}"),
        None => path.to_string(),
    }
}

/// "Cmd+Shift+2" → "⌘⇧2".
pub fn pretty_shortcut(s: &str) -> String {
    s.split('+')
        .map(|part| match part.trim().to_ascii_lowercase().as_str() {
            "cmd" | "command" | "super" | "meta" => "⌘".to_string(),
            "shift" => "⇧".to_string(),
            "alt" | "option" => "⌥".to_string(),
            "ctrl" | "control" => "⌃".to_string(),
            other => other.to_uppercase(),
        })
        .collect()
}

/// Builds a config shortcut ("Ctrl+Alt+Shift+Cmd+K") from a key press, or
/// `None` if it has no ⌘/⌃/⌥ or the key is not supported.
fn shortcut_string(flags: NSEventModifierFlags, key_code: u16) -> Option<String> {
    let has = |f| flags.contains(f);
    if !(has(NSEventModifierFlags::Command)
        || has(NSEventModifierFlags::Control)
        || has(NSEventModifierFlags::Option))
    {
        return None;
    }
    let key = key_name(key_code)?;
    let mut parts = Vec::new();
    if has(NSEventModifierFlags::Control) {
        parts.push("Ctrl");
    }
    if has(NSEventModifierFlags::Option) {
        parts.push("Alt");
    }
    if has(NSEventModifierFlags::Shift) {
        parts.push("Shift");
    }
    if has(NSEventModifierFlags::Command) {
        parts.push("Cmd");
    }
    parts.push(key);
    Some(parts.join("+"))
}

/// Physical (ANSI) key codes → names global-hotkey understands.
fn key_name(code: u16) -> Option<&'static str> {
    Some(match code {
        0 => "A",
        11 => "B",
        8 => "C",
        2 => "D",
        14 => "E",
        3 => "F",
        5 => "G",
        4 => "H",
        34 => "I",
        38 => "J",
        40 => "K",
        37 => "L",
        46 => "M",
        45 => "N",
        31 => "O",
        35 => "P",
        12 => "Q",
        15 => "R",
        1 => "S",
        17 => "T",
        32 => "U",
        9 => "V",
        13 => "W",
        7 => "X",
        16 => "Y",
        6 => "Z",
        29 => "0",
        18 => "1",
        19 => "2",
        20 => "3",
        21 => "4",
        23 => "5",
        22 => "6",
        26 => "7",
        28 => "8",
        25 => "9",
        49 => "Space",
        122 => "F1",
        120 => "F2",
        99 => "F3",
        118 => "F4",
        96 => "F5",
        97 => "F6",
        98 => "F7",
        100 => "F8",
        101 => "F9",
        109 => "F10",
        103 => "F11",
        111 => "F12",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_from_key_press_round_trips() {
        let f = NSEventModifierFlags::Command | NSEventModifierFlags::Shift;
        let s = shortcut_string(f, 19).unwrap();
        assert_eq!(s, "Shift+Cmd+2");
        assert!(global_hotkey::hotkey::HotKey::from_str(&s).is_ok());
        assert_eq!(pretty_shortcut(&s), "⇧⌘2");
        assert_eq!(pretty_shortcut("Cmd+Shift+2"), "⌘⇧2");
        assert!(
            shortcut_string(NSEventModifierFlags::Shift, 19).is_none(),
            "needs ⌘/⌃/⌥"
        );
        assert!(shortcut_string(NSEventModifierFlags::Command, 999).is_none());
    }

    #[test]
    fn home_is_abbreviated() {
        let home = dirs::home_dir().unwrap();
        let p = home.join("Pictures/Shots");
        assert_eq!(abbreviate_home(&p.to_string_lossy()), "~/Pictures/Shots");
    }

    #[test]
    fn every_hotkey_name_parses() {
        for code in 0..200u16 {
            if let Some(k) = key_name(code) {
                let s = format!("Cmd+{k}");
                assert!(global_hotkey::hotkey::HotKey::from_str(&s).is_ok(), "{s}");
            }
        }
    }
}
