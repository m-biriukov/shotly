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
    NSPopUpButton, NSTextField, NSView, NSWindow, NSWindowDelegate, NSWindowStyleMask,
};
use objc2_foundation::{
    NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, ns_string,
};
use winit::event_loop::EventLoopProxy;

use crate::app::AppEvent;
use crate::config::{Config, Format, ToastCorner, ToastOffset, ToastSize};
use crate::login;
use crate::style::{COLOR_NAMES, Width};

const SIZE_NAMES: [&str; 3] = ["Small", "Medium", "Large"];

struct Ivars {
    proxy: EventLoopProxy<AppEvent>,
    base: Config,
    hotkey: RefCell<String>,
    recording: Cell<bool>,
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
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    struct ScreenshootSettings;

    unsafe impl NSObjectProtocol for ScreenshootSettings {}

    unsafe impl NSWindowDelegate for ScreenshootSettings {
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

    impl ScreenshootSettings {
        #[unsafe(method(recordHotkey:))]
        fn record_hotkey(&self, _sender: Option<&AnyObject>) {
            self.ivars().recording.set(true);
            if let Some(b) = self.ivars().hotkey_button.get() {
                b.setTitle(ns_string!("Type shortcut… (Esc to cancel)"));
            }
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

impl ScreenshootSettings {
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
        cfg
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
        if iv.recording.get() {
            if code == 53 {
                // Esc: keep the old shortcut.
                self.finish_recording();
                return true;
            }
            if let Some(s) = shortcut_string(flags, code)
                && global_hotkey::hotkey::HotKey::from_str(&s).is_ok()
            {
                *iv.hotkey.borrow_mut() = s;
                self.finish_recording();
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

    fn finish_recording(&self) {
        self.ivars().recording.set(false);
        if let Some(b) = self.ivars().hotkey_button.get() {
            b.setTitle(&NSString::from_str(&pretty_shortcut(
                &self.ivars().hotkey.borrow(),
            )));
        }
    }
}

/// Owns the Settings window while it is open.
pub struct Settings {
    ctrl: Retained<ScreenshootSettings>,
}

impl Settings {
    pub fn open(cfg: &Config, proxy: EventLoopProxy<AppEvent>) -> Result<Self> {
        let mtm = MainThreadMarker::new().context("Settings must open on the main thread")?;
        let ctrl = ScreenshootSettings::alloc(mtm).set_ivars(Ivars {
            proxy,
            base: cfg.clone(),
            hotkey: RefCell::new(cfg.hotkey.clone()),
            recording: Cell::new(false),
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
        });
        // SAFETY: plain NSObject init of our subclass.
        let ctrl: Retained<ScreenshootSettings> = unsafe { msg_send![super(ctrl), init] };
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
const H: f64 = 530.0;
const LABEL_W: f64 = 150.0;
const X: f64 = 24.0 + LABEL_W + 12.0;

fn build(ctrl: &Retained<ScreenshootSettings>, cfg: &Config, mtm: MainThreadMarker) {
    let iv = ctrl.ivars();
    let target: &AnyObject = ctrl.as_ref();
    let window = new_window(ctrl, mtm);
    let view = window.contentView().expect("content view");

    // Cocoa's y axis points up; rows listed top to bottom.
    let (y_key, y_dir, y_name, y_fmt, y_color, y_size) = (
        H - 60.0,
        H - 96.0,
        H - 132.0,
        H - 186.0,
        H - 222.0,
        H - 258.0,
    );
    let (y_toast, y_pos, y_tsize, y_off, y_login) =
        (H - 306.0, H - 342.0, H - 378.0, H - 414.0, H - 458.0);

    label(&view, "Capture shortcut:", y_key, mtm);
    let hk = button(
        &pretty_shortcut(&cfg.hotkey),
        target,
        sel!(recordHotkey:),
        mtm,
    );
    place(&view, &hk, X, y_key, 250.0);

    label(&view, "Save folder:", y_dir, mtm);
    let folder = NSTextField::textFieldWithString(&NSString::from_str(&cfg.save_dir), mtm);
    place(&view, &folder, X, y_dir + 2.0, 210.0);
    place(
        &view,
        &button("Choose…", target, sel!(chooseFolder:), mtm),
        X + 216.0,
        y_dir,
        96.0,
    );

    label(&view, "File name:", y_name, mtm);
    let filename = NSTextField::textFieldWithString(&NSString::from_str(&cfg.filename), mtm);
    place(&view, &filename, X, y_name + 2.0, 310.0);
    let hint = NSTextField::labelWithString(ns_string!("{date} and {time} are filled in"), mtm);
    hint.setFont(Some(&NSFont::systemFontOfSize(11.0)));
    place(&view, &hint, X, y_name - 20.0, 310.0);

    label(&view, "Format:", y_fmt, mtm);
    let format = popup(
        &["PNG", "JPEG (quality 90)"],
        usize::from(cfg.format == Format::Jpeg),
        mtm,
    );
    place(&view, &format, X, y_fmt, 200.0);

    label(&view, "Default color:", y_color, mtm);
    let color = popup(&COLOR_NAMES, cfg.color, mtm);
    place(&view, &color, X, y_color, 200.0);

    label(&view, "Default size:", y_size, mtm);
    let size_idx = Width::ALL
        .iter()
        .position(|&w| w == cfg.width())
        .unwrap_or(1);
    let size = popup(&SIZE_NAMES, size_idx, mtm);
    place(&view, &size, X, y_size, 200.0);

    let toast = checkbox("Show notice after copy or save", cfg.toast, mtm);
    place(&view, &toast, X, y_toast, 310.0);

    label(&view, "Notice position:", y_pos, mtm);
    let names = ToastCorner::ALL.map(ToastCorner::name);
    let toast_position = popup(
        &names,
        index_of(&ToastCorner::ALL, &cfg.toast_position),
        mtm,
    );
    place(&view, &toast_position, X, y_pos, 200.0);

    label(&view, "Notice size:", y_tsize, mtm);
    let names = ToastSize::ALL.map(ToastSize::name);
    let toast_size = popup(&names, index_of(&ToastSize::ALL, &cfg.toast_size), mtm);
    place(&view, &toast_size, X, y_tsize, 200.0);

    label(&view, "Distance from edge:", y_off, mtm);
    let names = ToastOffset::ALL.map(ToastOffset::name);
    let toast_offset = popup(&names, index_of(&ToastOffset::ALL, &cfg.toast_offset), mtm);
    place(&view, &toast_offset, X, y_off, 200.0);
    place(
        &view,
        &button("Preview", target, sel!(previewToast:), mtm),
        X + 210.0,
        y_off,
        100.0,
    );

    let login_box = checkbox("Launch at login", login::is_enabled(), mtm);
    place(&view, &login_box, X, y_login, 310.0);

    let save = button("Save", target, sel!(save:), mtm);
    let cancel = button("Cancel", target, sel!(cancel:), mtm);
    save.setKeyEquivalent(ns_string!("\r"));
    cancel.setKeyEquivalent(ns_string!("\u{1b}"));
    place(&view, &save, W - 24.0 - 96.0, 20.0, 96.0);
    place(&view, &cancel, W - 24.0 - 96.0 - 8.0 - 96.0, 20.0, 96.0);

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
    window.center();
    let _ = iv.window.set(window);
}

/// The titled, closable Settings window, delegating to `ctrl`.
fn new_window(ctrl: &Retained<ScreenshootSettings>, mtm: MainThreadMarker) -> Retained<NSWindow> {
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
    window.setTitle(ns_string!("Screenshoot Settings"));
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

fn label(view: &NSView, text: &str, y: f64, mtm: MainThreadMarker) {
    let l = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    l.setAlignment(objc2_app_kit::NSTextAlignment::Right);
    place(view, &l, 24.0, y + 4.0, LABEL_W);
}

fn popup(items: &[&str], selected: usize, mtm: MainThreadMarker) -> Retained<NSPopUpButton> {
    let p = NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), NSRect::ZERO, false);
    for item in items {
        p.addItemWithTitle(&NSString::from_str(item));
    }
    p.selectItemAtIndex(selected.min(items.len() - 1) as isize);
    p
}

/// Adds `v` to `view` at (`x`, `y`) with width `w` and its natural height.
fn place(view: &NSView, v: &NSView, x: f64, y: f64, w: f64) {
    let h = v.fittingSize().height.max(22.0);
    v.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(w, h)));
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
