//! "Launch at login" via a per-user `LaunchAgent`. Unlike `SMAppService` this
//! works for a plain binary as well as an .app bundle.

use std::path::PathBuf;

use anyhow::{Context, Result};

const LABEL: &str = "dev.screenshoot.agent";

fn plist_path() -> Option<PathBuf> {
    Some(
        dirs::home_dir()?
            .join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist")),
    )
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
    fn plist_escapes_path() {
        let p = plist("/Apps/A&B <x>/screenshoot");
        assert!(p.contains("<string>/Apps/A&amp;B &lt;x&gt;/screenshoot</string>"));
        assert!(p.contains(LABEL));
    }
}
