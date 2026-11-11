//! "Launch at login" via a per-user `LaunchAgent`. Unlike `SMAppService` this
//! works for a plain binary as well as an .app bundle.

use std::path::PathBuf;

use anyhow::{Context, Result};

const LABEL: &str = "dev.shotly.agent";

fn plist_path() -> Option<PathBuf> {
    Some(
        dirs::home_dir()?
            .join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist")),
    )
}

/// Label of the launch agent from when the app was called Screenshoot.
const LEGACY_LABEL: &str = "dev.screenshoot.agent";

/// If "Launch at login" was on under the old name, its agent points at the
/// old app and no longer works: replace it with one for this app.
pub fn migrate_legacy() {
    let Some(dir) = dirs::home_dir().map(|h| h.join("Library/LaunchAgents")) else {
        return;
    };
    if !remove_legacy_agent(&dir) {
        return;
    }
    match set_enabled(true) {
        Ok(()) => println!("Moved Launch at Login from Screenshoot to Shotly"),
        Err(e) => eprintln!("launch at login (after rename): {e:#}"),
    }
}

/// Deletes the old agent's plist from `dir`. True if there was one.
fn remove_legacy_agent(dir: &std::path::Path) -> bool {
    let old = dir.join(format!("{LEGACY_LABEL}.plist"));
    if !old.exists() {
        return false;
    }
    match std::fs::remove_file(&old) {
        Ok(()) => true,
        Err(e) => {
            eprintln!("could not remove {}: {e}", old.display());
            false
        }
    }
}

pub fn is_enabled() -> bool {
    plist_path().is_some_and(|p| p.exists())
}

pub fn set_enabled(on: bool) -> Result<()> {
    let path = plist_path().context("no home folder")?;
    if !on {
        if path.exists() {
            std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        }
        return Ok(());
    }
    let exe = std::env::current_exe().context("finding own executable")?;
    let exe = exe.canonicalize().unwrap_or(exe);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, plist(&exe.to_string_lossy()))
        .with_context(|| format!("writing {}", path.display()))
}

fn plist(exe: &str) -> String {
    let exe = exe
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>ProcessType</key>
    <string>Interactive</string>
</dict>
</plist>
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_old_agent_is_removed_once() {
        let dir = std::env::temp_dir().join(format!("shotly-agents-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!remove_legacy_agent(&dir), "nothing to remove yet");
        std::fs::write(dir.join(format!("{LEGACY_LABEL}.plist")), "x").unwrap();
        assert!(remove_legacy_agent(&dir));
        assert!(!remove_legacy_agent(&dir), "gone after the first time");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn plist_escapes_path() {
        let p = plist("/Apps/A&B <x>/shotly");
        assert!(p.contains("<string>/Apps/A&amp;B &lt;x&gt;/shotly</string>"));
        assert!(p.contains(LABEL));
    }
}
