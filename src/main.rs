//! Screenshoot: a macOS menu-bar screenshot tool with annotations.
//!
//! `screenshoot`            run in the menu bar (global hotkey, Settings…)
//! `screenshoot --capture`  take one screenshot right away, then exit

mod app;
mod capture;
mod config;
mod editor;
mod geom;
mod keys;
mod layers;
mod login;
mod macos;
mod output;
mod overlay;
mod paint;
mod render;
mod settings;
mod shape;
mod snap;
mod style;
mod text;
mod theme;
mod toast;
mod toolbar;

use anyhow::Result;
use winit::event_loop::EventLoop;
use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};

fn main() -> Result<()> {
    let oneshot = std::env::args().skip(1).any(|a| a == "--capture");
    log_to_file_unless_terminal();
    // Two menu-bar copies would fight over the shortcut; keep the first one.
    let _lock = if oneshot {
        None
    } else {
        match single_instance_lock() {
            Lock::Held(f) => Some(f),
            Lock::Unavailable => None,
            Lock::TakenByOther => {
                println!("Screenshoot is already running in the menu bar.");
                return Ok(());
            }
        }
    };
    let event_loop = EventLoop::<app::AppEvent>::with_user_event()
        .with_activation_policy(ActivationPolicy::Accessory)
        .with_activate_ignoring_other_apps(oneshot)
        .build()?;
    let mut app = app::App::new(event_loop.create_proxy(), oneshot);
    event_loop.run_app(&mut app)?;
    Ok(())
}

enum Lock {
    Held(std::fs::File),
    TakenByOther,
    /// Could not create the lock file; run anyway.
    Unavailable,
}

/// An exclusive `flock` on a file next to the config, released by the OS
/// when the process exits (even if it crashes).
fn single_instance_lock() -> Lock {
    use std::os::fd::AsRawFd;
    let path = config::Config::path().with_file_name("screenshoot.lock");
    let file = path
        .parent()
        .and_then(|d| std::fs::create_dir_all(d).ok())
        .and_then(|()| {
            std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(&path)
                .ok()
        });
    let Some(file) = file else {
        return Lock::Unavailable;
    };
    // SAFETY: flock on a file descriptor we own.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        Lock::Held(file)
    } else {
        Lock::TakenByOther
    }
}

/// Apps started from Finder or at login have nowhere to print, so errors and
/// crashes would vanish. Send stdout/stderr to `~/Library/Logs/Screenshoot.log`
/// instead (owner-only; rotated past 1 MB). Terminal runs keep printing.
fn log_to_file_unless_terminal() {
    use std::io::Write;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;

    // SAFETY: isatty only inspects the descriptor.
    if unsafe { libc::isatty(libc::STDERR_FILENO) } == 1 {
        return;
    }
    let Some(dir) = dirs::home_dir().map(|h| h.join("Library/Logs")) else {
        return;
    };
    let path = dir.join("Screenshoot.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 1_000_000) {
        let _ = std::fs::rename(&path, dir.join("Screenshoot.old.log"));
    }
    let _ = std::fs::create_dir_all(&dir);
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&path)
    else {
        return;
    };
    let _ = writeln!(
        file,
        "--- {} Screenshoot {} started ---",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        env!("CARGO_PKG_VERSION")
    );
    // SAFETY: dup2 onto the standard descriptors; `file` stays open until
    // the end of this function, and the duplicates outlive it.
    unsafe {
        libc::dup2(file.as_raw_fd(), libc::STDOUT_FILENO);
        libc::dup2(file.as_raw_fd(), libc::STDERR_FILENO);
    }
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        eprintln!(
            "--- {} crash ---",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        );
        default(info);
    }));
}
