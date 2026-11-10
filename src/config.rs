//! User settings, stored as TOML in
//! `~/Library/Application Support/screenshoot/config.toml`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::style::{PALETTE, Width};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Png,
    Jpeg,
}

impl Format {
    pub fn ext(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
        }
    }
}

/// Screen corner the notice appears in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToastCorner {
    TopRight,
    TopLeft,
    BottomRight,
    BottomLeft,
}

impl ToastCorner {
    pub const ALL: [Self; 4] = [
        Self::TopRight,
        Self::TopLeft,
        Self::BottomRight,
        Self::BottomLeft,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::TopRight => "Top right",
            Self::TopLeft => "Top left",
            Self::BottomRight => "Bottom right",
            Self::BottomLeft => "Bottom left",
        }
    }

    pub fn is_right(self) -> bool {
        matches!(self, Self::TopRight | Self::BottomRight)
    }

    pub fn is_top(self) -> bool {
        matches!(self, Self::TopRight | Self::TopLeft)
    }
}

/// How big the notice is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToastSize {
    Small,
    Medium,
    Large,
}

impl ToastSize {
    pub const ALL: [Self; 3] = [Self::Small, Self::Medium, Self::Large];

    pub fn name(self) -> &'static str {
        match self {
            Self::Small => "Small",
            Self::Medium => "Medium",
            Self::Large => "Large",
        }
    }

    /// Multiplier for every size in the notice (Small is the original look).
    pub fn factor(self) -> f32 {
        match self {
            Self::Small => 1.0,
            Self::Medium => 1.4,
            Self::Large => 1.8,
        }
    }
}

/// Gap between the notice and the screen edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToastOffset {
    Small,
    Medium,
    Large,
}

impl ToastOffset {
    pub const ALL: [Self; 3] = [Self::Small, Self::Medium, Self::Large];

    pub fn name(self) -> &'static str {
        match self {
            Self::Small => "Small (16 pt)",
            Self::Medium => "Medium (32 pt)",
            Self::Large => "Large (56 pt)",
        }
    }

    pub fn pt(self) -> i32 {
        match self {
            Self::Small => 16,
            Self::Medium => 32,
            Self::Large => 56,
        }
    }
}

const DEFAULT_SAVE_DIR: &str = "~/Pictures/Screenshots";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Global shortcut, e.g. "Cmd+Shift+2".
    pub hotkey: String,
    /// Where Cmd+S saves; `~` expands to the home folder.
    pub save_dir: String,
    /// File name without extension. `{date}` → 2026-11-10, `{time}` → 14.03.27.
    pub filename: String,
    pub format: Format,
    /// Show the "Copied" / "Saved" notice.
    pub toast: bool,
    pub toast_position: ToastCorner,
    pub toast_size: ToastSize,
    pub toast_offset: ToastOffset,
    /// Palette index (0–7) to start with; updated to the last one used.
    pub color: usize,
    /// "S", "M" or "L"; updated to the last one used.
    pub width: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: "Cmd+Shift+2".into(),
            save_dir: DEFAULT_SAVE_DIR.into(),
            filename: "Screenshot {date} at {time}".into(),
            format: Format::Png,
            toast: true,
            toast_position: ToastCorner::TopRight,
            toast_size: ToastSize::Medium,
            toast_offset: ToastOffset::Medium,
            color: 0,
            width: "M".into(),
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("screenshoot")
            .join("config.toml")
    }

    /// Loads the config, falling back to defaults (and reporting why) if the
    /// file is missing or broken.
    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str::<Self>(&s)
                .map(Self::sanitized)
                .unwrap_or_else(|e| {
                    eprintln!(
                        "warning: {} is invalid, using defaults: {e}",
                        path.display()
                    );
                    Self::default()
                }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::path())
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let text = toml::to_string_pretty(self).context("serializing config")?;
        // Write a temp file and rename it over the old one, so a crash
        // mid-write never leaves a half-written config behind.
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))
    }

    fn sanitized(mut self) -> Self {
        if self.color >= PALETTE.len() {
            self.color = 0;
        }
        if parse_width(&self.width).is_none() {
            self.width = "M".into();
        }
        self
    }

    pub fn width(&self) -> Width {
        parse_width(&self.width).unwrap_or(Width::M)
    }

    pub fn set_width(&mut self, w: Width) {
        self.width = width_name(w).into();
    }

    /// The save folder as an absolute path (see [`resolve_dir`]).
    pub fn save_dir(&self) -> PathBuf {
        resolve_dir(&self.save_dir)
    }

    /// File name for a capture taken at `now`, with extension.
    pub fn file_name(&self, now: chrono::DateTime<chrono::Local>) -> String {
        let pattern = if self.filename.trim().is_empty() {
            "Screenshot {date} at {time}"
        } else {
            &self.filename
        };
        let name = pattern
            .replace("{date}", &now.format("%Y-%m-%d").to_string())
            .replace("{time}", &now.format("%H.%M.%S").to_string())
            .replace(['/', ':'], "-");
        format!("{name}.{}", self.format.ext())
    }
}

pub fn parse_width(s: &str) -> Option<Width> {
    match s.trim().to_ascii_uppercase().as_str() {
        "S" => Some(Width::S),
        "M" => Some(Width::M),
        "L" => Some(Width::L),
        _ => None,
    }
}

pub fn width_name(w: Width) -> &'static str {
    match w {
        Width::S => "S",
        Width::M => "M",
        Width::L => "L",
    }
}

/// Turns a folder from the config into an absolute path: `~` means the
/// home folder, a relative path is taken relative to it, and an empty one
/// falls back to the default. Absolute results also mean a folder named like
/// a command-line flag can never reach `open` as one.
pub fn resolve_dir(p: &str) -> PathBuf {
    let p = p.trim();
    let p = if p.is_empty() { DEFAULT_SAVE_DIR } else { p };
    let path = expand_tilde(p);
    match dirs::home_dir() {
        Some(home) if path.is_relative() => home.join(path),
        _ => path,
    }
}

fn expand_tilde(p: &str) -> PathBuf {
    match (p.strip_prefix("~"), dirs::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest.trim_start_matches('/')),
        _ => PathBuf::from(p),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn round_trips_through_toml() {
        let dir = std::env::temp_dir().join(format!("screenshoot-cfg-{}", std::process::id()));
        let path = dir.join("config.toml");
        let mut c = Config {
            format: Format::Jpeg,
            color: 4,
            ..Config::default()
        };
        c.set_width(Width::L);
        c.save_to(&path).unwrap();
        assert_eq!(Config::load_from(&path), c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn toast_settings_use_readable_names_and_default_when_missing() {
        let c: Config =
            toml::from_str("toast_position = \"bottom-left\"\ntoast_size = \"large\"").unwrap();
        assert_eq!(c.toast_position, ToastCorner::BottomLeft);
        assert_eq!(c.toast_size, ToastSize::Large);
        assert_eq!(c.toast_offset, ToastOffset::Medium, "missing key → default");
        let text = toml::to_string_pretty(&Config::default()).unwrap();
        assert!(text.contains("toast_position = \"top-right\""), "{text}");
        assert!(ToastSize::Large.factor() > ToastSize::Small.factor());
        assert!(ToastOffset::Large.pt() > ToastOffset::Small.pt());
    }

    #[test]
    fn missing_keys_use_defaults_and_bad_values_are_fixed() {
        let c: Config = toml::from_str("format = \"jpeg\"\ncolor = 99\nwidth = \"XL\"").unwrap();
        let c = c.sanitized();
        assert_eq!(c.format, Format::Jpeg);
        assert_eq!(c.hotkey, "Cmd+Shift+2");
        assert_eq!(c.color, 0);
        assert_eq!(c.width(), Width::M);
    }

    #[test]
    fn file_name_fills_pattern() {
        let now = chrono::Local
            .with_ymd_and_hms(2026, 11, 10, 9, 5, 7)
            .unwrap();
        let mut c = Config::default();
        assert_eq!(c.file_name(now), "Screenshot 2026-11-10 at 09.05.07.png");
        c.filename = "shot/{time}".into();
        c.format = Format::Jpeg;
        assert_eq!(c.file_name(now), "shot-09.05.07.jpg");
    }

    #[test]
    fn tilde_expands_to_home() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(expand_tilde("~/Pictures"), home.join("Pictures"));
        assert_eq!(expand_tilde("/tmp/x"), PathBuf::from("/tmp/x"));
    }

    #[test]
    fn save_dir_is_always_absolute() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(resolve_dir("/tmp/x"), PathBuf::from("/tmp/x"));
        assert_eq!(resolve_dir("~/Shots"), home.join("Shots"));
        assert_eq!(resolve_dir("Shots"), home.join("Shots"));
        assert_eq!(resolve_dir("-a"), home.join("-a"));
        assert_eq!(resolve_dir("  "), home.join("Pictures/Screenshots"));
        assert!(resolve_dir("x").is_absolute());
    }
}
