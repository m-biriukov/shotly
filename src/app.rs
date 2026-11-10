//! The long-running menu-bar app: tray icon and menu, global hotkey,
//! capture sessions, toasts and the Settings window.

use std::str::FromStr;
use std::time::{Duration, Instant};

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use objc2::rc::Retained;
use objc2_app_kit::NSRunningApplication;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::window::WindowId;

use crate::config::{self, Config};
use crate::overlay::{Finished, Outcome, Session};
use crate::settings::Settings;
use crate::toast::{Notice, OnClick, Step, Toast, ToastStyle};
use crate::{login, macos};

#[derive(Debug)]
pub enum AppEvent {
    HotKey,
    Menu(MenuId),
    /// The Settings window saved a new config (already written to disk).
    ConfigChanged(Config),
    SettingsClosed,
    /// The Settings window's "Preview" button: show a sample notice with
    /// these (not yet saved) settings.
    PreviewToast(Config),
}

/// Lets the tray menu close before the screen is frozen.
const MENU_CAPTURE_DELAY: Duration = Duration::from_millis(250);
const TOAST_SHORT: Duration = Duration::from_millis(1200);
const TOAST_LONG: Duration = Duration::from_millis(2500);

struct MenuItems {
    capture: MenuItem,
    settings: MenuItem,
    login: CheckMenuItem,
    quit: MenuItem,
}

pub struct App {
    proxy: EventLoopProxy<AppEvent>,
    /// `--capture`: take one screenshot, then exit (no menu bar icon).
    oneshot: bool,
    started: bool,
    cfg: Config,
    hotkeys: Option<GlobalHotKeyManager>,
    hotkey: Option<HotKey>,
    tray: Option<TrayIcon>,
    menu: Option<MenuItems>,
    session: Option<Session>,
    /// App that was frontmost before the capture, to hand focus back to.
    prev_app: Option<Retained<NSRunningApplication>>,
    pending_capture: Option<Instant>,
    toast: Option<Toast>,
    settings: Option<Settings>,
}

impl App {
    pub fn new(proxy: EventLoopProxy<AppEvent>, oneshot: bool) -> Self {
        Self {
            proxy,
            oneshot,
            started: false,
            cfg: Config::load(),
            hotkeys: None,
            hotkey: None,
            tray: None,
            menu: None,
            session: None,
            prev_app: None,
            pending_capture: None,
            toast: None,
            settings: None,
        }
    }

    fn setup_menu_bar(&mut self) -> anyhow::Result<()> {
        let capture = MenuItem::new("Capture Area", true, config_accelerator(&self.cfg.hotkey));
        let settings = MenuItem::new("Settings…", true, None);
        let login = CheckMenuItem::new("Launch at Login", true, login::is_enabled(), None);
        let quit = MenuItem::new("Quit Screenshoot", true, None);
        let menu = Menu::new();
        menu.append_items(&[
            &capture,
            &PredefinedMenuItem::separator(),
            &settings,
            &login,
            &PredefinedMenuItem::separator(),
            &quit,
        ])?;
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_icon_templated(tray_icon_image())
            .with_tooltip("Screenshoot")
            .build()?;
        self.tray = Some(tray);
        self.menu = Some(MenuItems {
            capture,
            settings,
            login,
            quit,
        });

        let proxy = self.proxy.clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let _ = proxy.send_event(AppEvent::Menu(e.id));
        }));
        let proxy = self.proxy.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
            if e.state == HotKeyState::Pressed {
                let _ = proxy.send_event(AppEvent::HotKey);
            }
        }));
        self.hotkeys = Some(GlobalHotKeyManager::new()?);
        Ok(())
    }

    /// (Re-)registers the configured shortcut. On failure (invalid, or
    /// reserved by another app) tells the user how to fix it.
    fn register_hotkey(&mut self, el: &ActiveEventLoop) {
        let Some(mgr) = &self.hotkeys else { return };
        let pretty = crate::settings::pretty_shortcut(&self.cfg.hotkey);
        let new = match HotKey::from_str(&self.cfg.hotkey) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("invalid hotkey {:?}: {e}", self.cfg.hotkey);
                return self.hotkey_problem(el, format!("Shortcut {pretty} is not valid"));
            }
        };
        if self.hotkey == Some(new) {
            return;
        }
        if let Some(old) = self.hotkey.take() {
            let _ = mgr.unregister(old);
        }
        let ok = match mgr.register(new) {
            Ok(()) => {
                self.hotkey = Some(new);
                true
            }
            Err(e) => {
                eprintln!("could not register hotkey {:?}: {e}", self.cfg.hotkey);
                false
            }
        };
        if let Some(m) = &self.menu {
            let accel = if ok {
                config_accelerator(&self.cfg.hotkey)
            } else {
                None
            };
            let _ = m.capture.set_accelerator(accel);
        }
        if !ok {
            self.hotkey_problem(el, format!("Shortcut {pretty} is used by another app"));
        }
    }

    fn hotkey_problem(&mut self, el: &ActiveEventLoop, title: String) {
        // Important: without it the app looks dead.
        let detail = "Choose another one in Settings… (menu bar icon)";
        self.notify(
            el,
            None,
            Notice::new(title, Duration::from_secs(6))
                .detail(detail)
                .important(),
        );
    }

    fn start_capture(&mut self, el: &ActiveEventLoop) {
        if self.session.is_some() {
            return;
        }
        // The toast would end up in the screenshot.
        self.toast = None;
        self.prev_app = macos::frontmost_other_app();
        match Session::start(el, &self.cfg) {
            Ok(s) => self.session = Some(s),
            Err(e) if e.is::<crate::capture::NoPermission>() => self.permission_problem(el),
            Err(e) => self.finish(el, None, Some(format!("{e:#}"))),
        }
    }

    fn permission_problem(&mut self, el: &ActiveEventLoop) {
        let who = if std::env::var_os("TERM").is_some() {
            "the app you started it from (your terminal)"
        } else {
            "Screenshoot"
        };
        eprintln!(
            "Screen Recording permission is missing. Enable {who} in System Settings › Privacy & Security › \
             Screen Recording, then restart Screenshoot."
        );
        if self.oneshot {
            return el.exit();
        }
        let notice = Notice::new("Screen Recording permission needed", Duration::from_secs(8))
            .detail("Click to open System Settings, then restart")
            .on_click(OnClick::PermissionSettings)
            .important();
        self.notify(el, None, notice);
    }

    fn finish(&mut self, el: &ActiveEventLoop, done: Option<Finished>, error: Option<String>) {
        self.session = None;
        if self.oneshot {
            if let Some(Outcome::Saved(p)) = done.as_ref().map(|d| &d.outcome) {
                println!("Saved {}", p.display());
            }
            if let Some(e) = error.or_else(|| match done.map(|d| d.outcome) {
                Some(Outcome::Failed(e)) => Some(e),
                _ => None,
            }) {
                eprintln!("error: {e}");
            }
            return el.exit();
        }
        // Hand focus back unless the user is working in our Settings window.
        if self.settings.is_none()
            && let Some(app) = self.prev_app.take()
        {
            macos::reactivate(&app);
        }
        let Some(done) = done else {
            let detail = error.unwrap_or_default();
            eprintln!("capture failed: {detail}");
            self.notify(
                el,
                None,
                Notice::new("Capture failed", TOAST_LONG).detail(detail),
            );
            return;
        };
        // Remember the last colour/size for next time.
        if done.color != self.cfg.color || config::parse_width(&self.cfg.width) != Some(done.width)
        {
            self.cfg.color = done.color;
            self.cfg.set_width(done.width);
            if let Err(e) = self.cfg.save() {
                eprintln!("could not save config: {e:#}");
            }
        }
        let display = Some(done.display);
        match done.outcome {
            Outcome::Cancelled => {}
            Outcome::Copied => {
                self.notify(el, display, Notice::new("Copied to clipboard", TOAST_SHORT));
            }
            Outcome::Saved(p) => {
                let name = p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let notice = Notice::new("Saved — click to show in Finder", TOAST_LONG)
                    .detail(name)
                    .on_click(OnClick::Reveal(p));
                self.notify(el, display, notice);
            }
            Outcome::Failed(e) => {
                eprintln!("error: {e}");
                self.notify(
                    el,
                    display,
                    Notice::new("Could not finish screenshot", TOAST_LONG).detail(e),
                );
            }
        }
    }

    /// Shows `n` on `display` (or the main display). Skipped when notices
    /// are turned off in Settings, unless it is important.
    fn notify(&mut self, el: &ActiveEventLoop, display: Option<(i32, i32, u32, u32)>, n: Notice) {
        if !self.cfg.toast && !n.important {
            return;
        }
        let style = ToastStyle::from(&self.cfg);
        self.show_notice(el, display, style, n);
    }

    fn show_notice(
        &mut self,
        el: &ActiveEventLoop,
        display: Option<(i32, i32, u32, u32)>,
        style: ToastStyle,
        n: Notice,
    ) {
        let display = display.or_else(|| {
            let m = el.primary_monitor()?;
            let s = m.scale_factor();
            let pos = m.position().to_logical::<i32>(s);
            let size = m.size().to_logical::<u32>(s);
            Some((pos.x, pos.y, size.width, size.height))
        });
        let Some(display) = display else { return };
        match Toast::show(el, display, style, n) {
            Ok(t) => self.toast = Some(t),
            Err(e) => eprintln!("toast failed: {e:#}"),
        }
    }

    fn open_settings(&mut self) {
        if let Some(s) = &self.settings {
            s.show();
            return;
        }
        match Settings::open(&self.cfg, self.proxy.clone()) {
            Ok(s) => self.settings = Some(s),
            Err(e) => eprintln!("could not open Settings: {e:#}"),
        }
    }

    fn apply_config(&mut self, el: &ActiveEventLoop, cfg: Config) {
        self.cfg = cfg;
        self.register_hotkey(el);
        if let Some(m) = &self.menu {
            m.login.set_checked(login::is_enabled());
        }
    }

    fn menu_event(&mut self, el: &ActiveEventLoop, id: &MenuId) {
        let Some(m) = &self.menu else { return };
        if id == m.capture.id() {
            self.pending_capture = Some(Instant::now() + MENU_CAPTURE_DELAY);
        } else if id == m.settings.id() {
            self.open_settings();
        } else if id == m.login.id() {
            let want = m.login.is_checked();
            if let Err(e) = login::set_enabled(want) {
                eprintln!("launch at login: {e:#}");
            }
            m.login.set_checked(login::is_enabled());
            if let Some(s) = &self.settings {
                s.set_login_checked(login::is_enabled());
            }
        } else if id == m.quit.id() {
            el.exit();
        }
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.started {
            return;
        }
        self.started = true;
        if self.oneshot {
            self.start_capture(el);
        } else if let Err(e) = self.setup_menu_bar() {
            eprintln!("error: could not set up menu bar: {e:#}");
            el.exit();
        } else {
            println!(
                "Screenshoot is running in the menu bar. Shortcut: {}",
                self.cfg.hotkey
            );
            crate::capture::warm_up();
            self.register_hotkey(el);
            // Dev aid (only with `--features dev-hooks`): SCREENSHOOT_TEST_SETTINGS=1
            // opens the Settings window right away.
            if cfg!(feature = "dev-hooks")
                && std::env::var_os("SCREENSHOOT_TEST_SETTINGS").is_some()
            {
                self.open_settings();
            }
            // Dev aid (only with `--features dev-hooks`): SCREENSHOOT_TEST_TOAST=1
            // shows a sample notice right away.
            if cfg!(feature = "dev-hooks") && std::env::var_os("SCREENSHOOT_TEST_TOAST").is_some() {
                let notice = Notice::new("Saved — click to show in Finder", Duration::from_secs(4))
                    .detail("Screenshot 2026-11-10 at 12.00.00.png");
                self.show_notice(el, None, ToastStyle::from(&self.cfg), notice);
            }
            // Dev aid (only with `--features dev-hooks`): SCREENSHOOT_TEST_CAPTURE=1
            // starts a capture after 1 s, through the same path as the menu item.
            if cfg!(feature = "dev-hooks") && std::env::var_os("SCREENSHOOT_TEST_CAPTURE").is_some()
            {
                self.pending_capture = Some(Instant::now() + Duration::from_secs(1));
            }
        }
    }

    fn user_event(&mut self, el: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::HotKey => self.start_capture(el),
            AppEvent::Menu(id) => self.menu_event(el, &id),
            AppEvent::ConfigChanged(cfg) => self.apply_config(el, cfg),
            AppEvent::PreviewToast(cfg) => {
                let notice = Notice::new("Saved — click to show in Finder", TOAST_LONG)
                    .detail("Screenshot 2026-11-10 at 12.00.00.png");
                self.show_notice(el, None, ToastStyle::from(&cfg), notice);
            }
            AppEvent::SettingsClosed => self.settings = None,
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if let Some(s) = &mut self.session
            && s.owns(id)
        {
            if let Some(done) = s.window_event(id, event) {
                self.finish(el, Some(done), None);
            }
            return;
        }
        if let Some(t) = &mut self.toast
            && t.owns(id)
        {
            match event {
                WindowEvent::RedrawRequested => t.redraw(),
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    ..
                } => {
                    match t.on_click() {
                        Some(OnClick::Reveal(p)) => macos::reveal_in_finder(p),
                        Some(OnClick::PermissionSettings) => {
                            crate::capture::open_permission_settings();
                        }
                        None => {}
                    }
                    self.toast = None;
                }
                _ => {}
            }
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if let Some(s) = &mut self.session {
            s.flush();
        }
        let now = Instant::now();
        if self.pending_capture.is_some_and(|t| t <= now) {
            self.pending_capture = None;
            self.start_capture(el);
        }
        // Move the notice along its animation; it asks when to be woken next.
        let toast_wake = match self.toast.as_mut().map(|t| t.step(now)) {
            Some(Step::Wait(at)) => Some(at),
            Some(Step::Done) => {
                self.toast = None;
                None
            }
            None => None,
        };
        let next = [
            self.pending_capture,
            toast_wake,
            self.session
                .as_ref()
                .and_then(super::overlay::Session::next_deadline),
        ]
        .into_iter()
        .flatten()
        .min();
        el.set_control_flow(match next {
            Some(t) => ControlFlow::WaitUntil(t),
            None => ControlFlow::Wait,
        });
    }
}

/// Menu shows the shortcut next to "Capture Area" (display only).
fn config_accelerator(hotkey: &str) -> Option<tray_icon::menu::accelerator::Accelerator> {
    hotkey.parse().ok()
}

/// Template image: viewfinder corners, 22 pt at 2x.
fn tray_icon_image() -> Icon {
    let size = 44u32;
    let mut pm = tiny_skia::Pixmap::new(size, size).unwrap();
    let paint = crate::paint::paint(0x000000, 255);
    let stroke = crate::paint::round_stroke(3.5);
    let (a, b, l) = (8.0f32, 36.0f32, 9.0f32);
    let mut pb = tiny_skia::PathBuilder::new();
    for (x, y, dx, dy) in [
        (a, a, 1.0, 1.0),
        (b, a, -1.0, 1.0),
        (a, b, 1.0, -1.0),
        (b, b, -1.0, -1.0),
    ] {
        pb.move_to(x, y + dy * l);
        pb.line_to(x, y);
        pb.line_to(x + dx * l, y);
    }
    pm.stroke_path(
        &pb.finish().unwrap(),
        &paint,
        &stroke,
        tiny_skia::Transform::identity(),
        None,
    );
    // Arrow across the frame, like the app icon.
    let id = tiny_skia::Transform::identity();
    let mut shaft = tiny_skia::PathBuilder::new();
    shaft.move_to(15.0, 29.0);
    shaft.line_to(25.0, 19.0);
    pm.stroke_path(
        &shaft.finish().unwrap(),
        &paint,
        &crate::paint::round_stroke(4.5),
        id,
        None,
    );
    let mut head = tiny_skia::PathBuilder::new();
    head.move_to(31.0, 13.0);
    head.line_to(21.0, 15.5);
    head.line_to(28.5, 23.0);
    head.close();
    pm.fill_path(
        &head.finish().unwrap(),
        &paint,
        tiny_skia::FillRule::Winding,
        id,
        None,
    );
    // Demultiply: tray-icon expects straight RGBA.
    let rgba = pm.pixels().iter().flat_map(|p| {
        let c = p.demultiply();
        [c.red(), c.green(), c.blue(), c.alpha()]
    });
    Icon::from_rgba(rgba.collect(), size, size).expect("valid icon")
}
