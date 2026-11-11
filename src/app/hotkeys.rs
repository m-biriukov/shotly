//! The two global shortcuts: registering them, and telling the user when one
//! cannot be used.

use std::str::FromStr;
use std::time::Duration;

use global_hotkey::GlobalHotKeyManager;
use global_hotkey::hotkey::HotKey;
use winit::event_loop::ActiveEventLoop;

use super::App;
use crate::toast::Notice;

impl App {
    /// (Re-)registers both configured shortcuts. On failure (invalid, or
    /// reserved by another app) tells the user how to fix it.
    pub(super) fn register_hotkeys(&mut self, el: &ActiveEventLoop) {
        let Some(mgr) = &self.hotkeys else { return };
        let mut problems = Vec::new();
        let (capture, record) = (self.cfg.hotkey.clone(), self.cfg.record_hotkey.clone());
        // Free both first, so swapping the two shortcuts cannot collide.
        for slot in [&mut self.hotkey, &mut self.record_hotkey] {
            if let Some(old) = slot.take() {
                let _ = mgr.unregister(old);
            }
        }
        let same = same_shortcut(&capture, &record);
        if same {
            problems.push("The capture and recording shortcuts are the same".to_string());
        }
        for (name, shortcut, slot) in [
            ("capture", capture, &mut self.hotkey),
            ("recording", record, &mut self.record_hotkey),
        ] {
            let pretty = crate::settings::pretty_shortcut(&shortcut);
            if name == "recording" && same {
                continue;
            }
            match sync_hotkey(mgr, &shortcut, slot) {
                HotkeyStatus::Unchanged | HotkeyStatus::Registered => {}
                HotkeyStatus::Invalid => {
                    problems.push(format!("The {name} shortcut {pretty} is not valid"));
                }
                HotkeyStatus::Taken => {
                    problems.push(format!(
                        "The {name} shortcut {pretty} is used by another app"
                    ));
                }
            }
        }
        if let Some(m) = &self.menu {
            let accel = |slot: &Option<HotKey>, shortcut: &str| {
                slot.and_then(|_| config_accelerator(shortcut))
            };
            let _ = m
                .capture
                .set_accelerator(accel(&self.hotkey, &self.cfg.hotkey));
            let _ = m
                .record
                .set_accelerator(accel(&self.record_hotkey, &self.cfg.record_hotkey));
        }
        if let Some(first) = problems.into_iter().next() {
            self.hotkey_problem(el, first);
        }
    }

    pub(super) fn hotkey_problem(&mut self, el: &ActiveEventLoop, title: String) {
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
}

enum HotkeyStatus {
    Unchanged,
    Registered,
    Invalid,
    Taken,
}

/// Makes `slot` hold `shortcut` as the registered global hotkey.
fn sync_hotkey(
    mgr: &GlobalHotKeyManager,
    shortcut: &str,
    slot: &mut Option<HotKey>,
) -> HotkeyStatus {
    let new = match HotKey::from_str(shortcut) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("invalid hotkey {shortcut:?}: {e}");
            return HotkeyStatus::Invalid;
        }
    };
    if *slot == Some(new) {
        return HotkeyStatus::Unchanged;
    }
    if let Some(old) = slot.take() {
        let _ = mgr.unregister(old);
    }
    match mgr.register(new) {
        Ok(()) => {
            *slot = Some(new);
            HotkeyStatus::Registered
        }
        Err(e) => {
            eprintln!("could not register hotkey {shortcut:?}: {e}");
            HotkeyStatus::Taken
        }
    }
}

/// Menu shows the shortcut next to "Capture Area" (display only).
pub(super) fn config_accelerator(
    hotkey: &str,
) -> Option<tray_icon::menu::accelerator::Accelerator> {
    hotkey.parse().ok()
}

/// Whether two shortcut strings are the same key combination as far as can be
/// told without parsing them (case and spaces around do not matter).
fn same_shortcut(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_shortcut_is_found_whatever_its_case() {
        assert!(same_shortcut("Cmd+Shift+2", " cmd+shift+2 "));
        assert!(!same_shortcut("Cmd+Shift+2", "Cmd+Shift+1"));
    }

    #[test]
    fn config_shortcuts_become_menu_accelerators() {
        assert!(config_accelerator("Cmd+Shift+2").is_some());
        assert!(config_accelerator("nonsense").is_none());
    }
}
