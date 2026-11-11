//! The long-running menu-bar app: tray icon and menu, global hotkey,
//! capture sessions, toasts and the Settings window.

use std::time::{Duration, Instant};

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use objc2::rc::Retained;
use objc2_app_kit::NSRunningApplication;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::window::WindowId;

use crate::config::{self, Config};
use crate::overlay::{Finished, Mode, Outcome, Session};
use crate::settings::Settings;
use crate::toast::{Notice, OnClick, Step, Toast, ToastStyle};
use crate::tray::tray_icon_image;
use crate::{login, macos};

mod flow;
mod hotkeys;
mod recording;

use flow::{Cmd, Flow, Input};
use hotkeys::config_accelerator;
use recording::{Active, Counting, Finishing};

#[derive(Debug)]
pub enum AppEvent {
    /// A global shortcut was pressed (the `HotKey::id`).
    HotKey(u32),
    Menu(MenuId),
    /// The Settings window saved a new config (already written to disk).
    ConfigChanged(Config),
    SettingsClosed,
    /// The Settings window's "Preview" button: show a sample notice with
    /// these (not yet saved) settings.
    PreviewToast(Config),
    /// The recording ended (stopped, or failed on its own); see `Recording::poll`.
    RecordingDone,
}

/// Lets the tray menu close before the screen is frozen.
const MENU_CAPTURE_DELAY: Duration = Duration::from_millis(250);
const TOAST_SHORT: Duration = Duration::from_millis(1200);
const TOAST_LONG: Duration = Duration::from_millis(2500);

struct MenuItems {
    capture: MenuItem,
    record: MenuItem,
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
    record_hotkey: Option<HotKey>,
    tray: Option<TrayIcon>,
    menu: Option<MenuItems>,
    session: Option<Session>,
    /// App that was frontmost before the capture, to hand focus back to.
    prev_app: Option<Retained<NSRunningApplication>>,
    pending_capture: Option<(Instant, Mode)>,
    /// What the app is doing (see `flow`); `session`, `counting` and
    /// `recording` hold the matching data.
    flow: Flow,
    counting: Option<Counting>,
    recording: Option<Active>,
    /// Recordings that failed on their own; kept (and stopped) until they
    /// report, so no stream or delegate is freed while still in use.
    finishing: Vec<Finishing>,
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
            record_hotkey: None,
            tray: None,
            menu: None,
            session: None,
            prev_app: None,
            pending_capture: None,
            flow: Flow::new(),
            counting: None,
            recording: None,
            finishing: Vec::new(),
            toast: None,
            settings: None,
        }
    }

    fn setup_menu_bar(&mut self) -> anyhow::Result<()> {
        let capture = MenuItem::new("Capture Area", true, config_accelerator(&self.cfg.hotkey));
        let record = MenuItem::new(
            "Record Screen",
            true,
            config_accelerator(&self.cfg.record_hotkey),
        );
        let settings = MenuItem::new("Settings…", true, None);
        let login = CheckMenuItem::new("Launch at Login", true, login::is_enabled(), None);
        let quit = MenuItem::new("Quit Shotly", true, None);
        let menu = Menu::new();
        menu.append_items(&[
            &capture,
            &record,
            &PredefinedMenuItem::separator(),
            &settings,
            &login,
            &PredefinedMenuItem::separator(),
            &quit,
        ])?;
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_icon_templated(tray_icon_image())
            .with_tooltip("Shotly")
            .build()?;
        self.tray = Some(tray);
        self.menu = Some(MenuItems {
            capture,
            record,
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
                let _ = proxy.send_event(AppEvent::HotKey(e.id));
            }
        }));
        self.hotkeys = Some(GlobalHotKeyManager::new()?);
        Ok(())
    }

    /// The capture shortcut (or a menu item, after its delay).
    fn start_capture(&mut self, el: &ActiveEventLoop, mode: Mode) {
        let cmds = self
            .flow
            .step(Input::CaptureRequested(mode), self.cfg.record_countdown);
        self.run_picker_cmds(el, &cmds);
    }

    /// Carries out the commands that open a picker or refuse a screenshot.
    fn run_picker_cmds(&mut self, el: &ActiveEventLoop, cmds: &[Cmd]) {
        for cmd in cmds {
            match *cmd {
                Cmd::OpenPicker(mode) => self.open_picker(el, mode),
                Cmd::ShowBusyNotice => {
                    // The freeze-frame overlay would end up in the video.
                    let notice = Notice::new("Recording in progress", TOAST_SHORT)
                        .detail("Stop it first to take a screenshot")
                        .important();
                    self.notify(el, None, notice);
                }
                _ => {}
            }
        }
    }

    fn open_picker(&mut self, el: &ActiveEventLoop, mode: Mode) {
        // The toast would end up in the screenshot.
        self.toast = None;
        self.prev_app = macos::frontmost_other_app();
        match Session::start(el, &self.cfg, mode) {
            Ok(s) => self.session = Some(s),
            Err(e) if e.is::<crate::capture::NoPermission>() => {
                self.flow.step(Input::PickerClosed { picked: false }, false);
                self.permission_problem(el);
            }
            Err(e) => self.finish(el, None, Some(format!("{e:#}"))),
        }
    }

    /// The data the app holds agrees with what the flow says is going on
    /// (checked in debug builds, between events).
    fn flow_matches_data(&self) -> bool {
        use flow::Phase::{Counting, Idle, Picking, Recording, Starting, Stopping};
        match self.flow.phase() {
            Idle => self.session.is_none() && self.counting.is_none() && self.recording.is_none(),
            Picking(_) => self.session.is_some(),
            Counting => self.counting.is_some(),
            Recording | Stopping => self.recording.is_some(),
            // Only while the stream starts, inside one event handler.
            Starting => false,
        }
    }

    fn permission_problem(&mut self, el: &ActiveEventLoop) {
        let who = if std::env::var_os("TERM").is_some() {
            "the app you started it from (your terminal)"
        } else {
            "Shotly"
        };
        eprintln!(
            "Screen Recording permission is missing. Enable {who} in System Settings › Privacy & Security › \
             Screen Recording, then restart Shotly."
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
        let picked = matches!(done.as_ref().map(|d| &d.outcome), Some(Outcome::Record(_)));
        let cmds = self
            .flow
            .step(Input::PickerClosed { picked }, self.cfg.record_countdown);
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
        self.remember_choices(&done);
        if let Outcome::Record(target) = &done.outcome {
            let target = *target;
            return self.begin_recording(el, target, &cmds);
        }
        let display = Some(done.display);
        match done.outcome {
            Outcome::Cancelled | Outcome::Record(_) => {}
            Outcome::Copied => {
                self.notify(el, display, Notice::new("Copied to clipboard", TOAST_SHORT));
            }
            Outcome::Saved(p) => {
                let notice = saved_notice("Saved — click to show in Finder", p);
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

    /// Remembers the last colour, size and preset for next time.
    fn remember_choices(&mut self, done: &Finished) {
        if !choices_changed(&self.cfg, done) {
            return;
        }
        self.cfg.color = done.color;
        self.cfg.set_width(done.width);
        self.cfg.preset = done.preset;
        if let Err(e) = self.cfg.save() {
            eprintln!("could not save config: {e:#}");
        }
    }

    /// Shows `n` on `display` (or the main display). Skipped when notices
    /// are turned off in Settings, unless it is important.
    fn notify(&mut self, el: &ActiveEventLoop, display: Option<macos::Frame>, n: Notice) {
        if !self.cfg.toast && !n.important {
            return;
        }
        let style = ToastStyle::from(&self.cfg);
        self.show_notice(el, display, style, n);
    }

    fn show_notice(
        &mut self,
        el: &ActiveEventLoop,
        display: Option<macos::Frame>,
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

    // ---- recording ----

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
        self.register_hotkeys(el);
        if let Some(m) = &self.menu {
            m.login.set_checked(login::is_enabled());
        }
    }

    fn menu_event(&mut self, el: &ActiveEventLoop, id: &MenuId) {
        let Some(m) = &self.menu else { return };
        if id == m.capture.id() {
            self.pending_capture = Some((Instant::now() + MENU_CAPTURE_DELAY, Mode::Screenshot));
        } else if id == m.record.id() {
            if self.flow.running() {
                self.toggle_recording(el);
            } else {
                self.pending_capture = Some((Instant::now() + MENU_CAPTURE_DELAY, Mode::Record));
            }
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
            self.quit(el);
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
            self.start_capture(el, Mode::Screenshot);
        } else if let Err(e) = self.setup_menu_bar() {
            eprintln!("error: could not set up menu bar: {e:#}");
            el.exit();
        } else {
            println!(
                "Shotly is running in the menu bar. Shortcut: {}",
                self.cfg.hotkey
            );
            crate::capture::warm_up();
            self.register_hotkeys(el);
            // Dev aid (only with `--features dev-hooks`): SHOTLY_TEST_SETTINGS=1
            // opens the Settings window right away.
            if cfg!(feature = "dev-hooks") && std::env::var_os("SHOTLY_TEST_SETTINGS").is_some() {
                self.open_settings();
            }
            // Dev aid (only with `--features dev-hooks`): SHOTLY_TEST_TOAST=1
            // shows a sample notice right away.
            if cfg!(feature = "dev-hooks") && std::env::var_os("SHOTLY_TEST_TOAST").is_some() {
                let notice = Notice::new("Saved — click to show in Finder", Duration::from_secs(4))
                    .detail("Screenshot 2026-11-10 at 12.00.00.png");
                self.show_notice(el, None, ToastStyle::from(&self.cfg), notice);
            }
            // Dev aid (only with `--features dev-hooks`): SHOTLY_TEST_CAPTURE=1
            // starts a capture after 1 s, through the same path as the menu item.
            if cfg!(feature = "dev-hooks") && std::env::var_os("SHOTLY_TEST_CAPTURE").is_some() {
                self.pending_capture =
                    Some((Instant::now() + Duration::from_secs(1), Mode::Screenshot));
            }
            // Dev aid (only with `--features dev-hooks`): SHOTLY_TEST_RECORD=1
            // starts the area picker for a recording after 1 s.
            if cfg!(feature = "dev-hooks") && std::env::var_os("SHOTLY_TEST_RECORD").is_some() {
                self.pending_capture =
                    Some((Instant::now() + Duration::from_secs(1), Mode::Record));
            }
        }
    }

    fn user_event(&mut self, el: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::HotKey(id) => {
                if self.record_hotkey.is_some_and(|h| h.id() == id) {
                    self.toggle_recording(el);
                } else if self.hotkey.is_some_and(|h| h.id() == id) {
                    self.start_capture(el, Mode::Screenshot);
                }
            }
            AppEvent::Menu(id) => self.menu_event(el, &id),
            AppEvent::ConfigChanged(cfg) => self.apply_config(el, cfg),
            AppEvent::PreviewToast(cfg) => {
                let notice = Notice::new("Saved — click to show in Finder", TOAST_LONG)
                    .detail("Screenshot 2026-11-10 at 12.00.00.png");
                self.show_notice(el, None, ToastStyle::from(&cfg), notice);
            }
            AppEvent::SettingsClosed => self.settings = None,
            AppEvent::RecordingDone => self.recording_done(el),
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
        debug_assert!(self.flow_matches_data(), "{:?}", self.flow.phase());
        if let Some(s) = &mut self.session {
            s.flush();
        }
        let now = Instant::now();
        if let Some((at, mode)) = self.pending_capture
            && at <= now
        {
            self.pending_capture = None;
            self.start_capture(el, mode);
        }
        let recording_wake = self.tick_recording(el, now);
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
            self.pending_capture.map(|(at, _)| at),
            recording_wake,
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

/// Whether the colour, size or preset the session ended with differs from
/// what the config remembers.
fn choices_changed(cfg: &Config, done: &Finished) -> bool {
    done.color != cfg.color
        || config::parse_width(&cfg.width) != Some(done.width)
        || done.preset != cfg.preset
}

/// "Saved": the file's name, and a click shows it in Finder.
fn saved_notice(title: &str, path: std::path::PathBuf) -> Notice {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Notice::new(title, TOAST_LONG)
        .detail(name)
        .on_click(OnClick::Reveal(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Preset;
    use crate::style::Width;

    fn finished(color: usize, width: Width, preset: Preset) -> Finished {
        Finished {
            outcome: Outcome::Cancelled,
            color,
            width,
            preset,
            display: (0, 0, 100, 100),
        }
    }

    #[test]
    fn only_a_changed_colour_size_or_preset_is_saved() {
        let cfg = Config::default();
        let same = finished(cfg.color, cfg.width(), cfg.preset);
        assert!(!choices_changed(&cfg, &same));
        assert!(choices_changed(&cfg, &finished(3, cfg.width(), cfg.preset)));
        assert!(choices_changed(
            &cfg,
            &finished(cfg.color, Width::L, cfg.preset)
        ));
        assert!(choices_changed(
            &cfg,
            &finished(cfg.color, cfg.width(), Preset::Full)
        ));
    }
}
