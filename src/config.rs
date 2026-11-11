//! User settings, stored as TOML in
//! `~/Library/Application Support/shotly/config.toml`.

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

/// What the capture picker offers first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Preset {
    /// The whole display.
    Full,
    /// Click a window.
    Window,
    /// Drag an area with the mouse.
    Custom,
}

impl Preset {
    pub const ALL: [Self; 3] = [Self::Full, Self::Window, Self::Custom];

    pub fn name(self) -> &'static str {
        match self {
            Self::Full => "Full screen",
            Self::Window => "Window",
            Self::Custom => "Custom",
        }
    }

    /// Keyboard shortcut in the picker.
    pub fn key(self) -> char {
        match self {
            Self::Full => 'F',
            Self::Window => 'W',
            Self::Custom => 'C',
        }
    }

    pub fn from_key(c: char) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|p| p.key().eq_ignore_ascii_case(&c))
    }
}

/// How big the cursor highlight is while recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HighlightSize {
    Small,
    Medium,
    Large,
}

impl HighlightSize {
    pub const ALL: [Self; 3] = [Self::Small, Self::Medium, Self::Large];

    pub fn name(self) -> &'static str {
        match self {
            Self::Small => "Small",
            Self::Medium => "Medium",
            Self::Large => "Large",
        }
    }

    /// Radius of the halo around the pointer, in points.
    pub fn radius_pt(self) -> f32 {
        match self {
            Self::Small => 18.0,
            Self::Medium => 28.0,
            Self::Large => 40.0,
        }
    }
}

/// Frame rates offered for recordings.
pub const FRAME_RATES: [u32; 2] = [30, 60];

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
    /// Last preset used in the picker (Full screen, Window or Custom).
    pub preset: Preset,
    /// Global shortcut that starts and stops a screen recording.
    pub record_hotkey: String,
    /// 30 or 60.
    pub record_fps: u32,
    /// Record at the display's full pixel resolution (otherwise points).
    pub record_retina: bool,
    /// Add the microphone to the recording.
    pub record_mic: bool,
    /// Count 3-2-1 before the recording starts.
    pub record_countdown: bool,
    /// Draw a frame around the recorded area while recording.
    pub record_outline: bool,
    /// Halo around the pointer.
    pub highlight_cursor: bool,
    /// Palette index of the halo.
    pub highlight_color: usize,
    pub highlight_size: HighlightSize,
    /// Ripple at every mouse click.
    pub show_clicks: bool,
    /// Palette indices of the ripples for the left and the right button.
    pub left_click_color: usize,
    pub right_click_color: usize,
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
            preset: Preset::Custom,
            record_hotkey: "Cmd+Shift+1".into(),
            record_fps: 30,
            record_retina: true,
            record_mic: false,
            record_countdown: true,
            record_outline: true,
            highlight_cursor: true,
            highlight_color: 2,
            highlight_size: HighlightSize::Medium,
            show_clicks: true,
            left_click_color: 0,
            right_click_color: 4,
        }
    }
}

impl Config {
    /// Where the app kept its files before it was renamed to Shotly.
    fn legacy_path() -> PathBuf {
        Self::path()
            .parent()
            .and_then(Path::parent)
            .unwrap_or_else(|| Path::new("."))
            .join("screenshoot")
            .join("config.toml")
    }

    /// Carries the settings over from the old name, once: if there is no
    /// Shotly config yet but a `screenshoot` one, it is copied.
    pub fn migrate_legacy() {
        match migrate_file(&Self::legacy_path(), &Self::path()) {
            Ok(true) => println!("Copied settings from the old Screenshoot folder"),
            Ok(false) => {}
            Err(e) => eprintln!("could not copy the old settings: {e:#}"),
        }
    }

    pub fn path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("shotly")
            .join("config.toml")
    }

    /// Loads the config, falling back to defaults (and reporting why) if the
    /// file is missing or broken.
    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(s) => Self::parse_lenient(&s)
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

    /// Parses `text`; a setting with an invalid value (a typo in a hand-edited
    /// file) is skipped, with a warning, instead of throwing away the rest.
    /// Only a file that is not valid TOML at all fails.
    fn parse_lenient(text: &str) -> Result<Self, toml::de::Error> {
        if let Ok(c) = toml::from_str::<Self>(text) {
            return Ok(c);
        }
        let user: toml::Table = toml::from_str(text)?;
        let mut merged = toml::Table::try_from(Self::default()).expect("default config serializes");
        for (key, value) in user {
            let mut trial = merged.clone();
            trial.insert(key.clone(), value);
            if trial.clone().try_into::<Self>().is_ok() {
                merged = trial;
            } else {
                eprintln!("warning: ignoring invalid setting {key:?} in the config");
            }
        }
        merged.try_into()
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
        let defaults = Self::default();
        for (c, d) in [
            (&mut self.highlight_color, defaults.highlight_color),
            (&mut self.left_click_color, defaults.left_click_color),
            (&mut self.right_click_color, defaults.right_click_color),
        ] {
            if *c >= PALETTE.len() {
                *c = d;
            }
        }
        if !FRAME_RATES.contains(&self.record_fps) {
            self.record_fps = defaults.record_fps;
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
        format!("{}.{}", self.file_stem(now), self.format.ext())
    }

    /// File name for a screen recording started at `now`.
    pub fn video_name(&self, now: chrono::DateTime<chrono::Local>) -> String {
        format!("{}.mp4", self.file_stem(now))
    }

    fn file_stem(&self, now: chrono::DateTime<chrono::Local>) -> String {
        let pattern = if self.filename.trim().is_empty() {
            "Screenshot {date} at {time}"
        } else {
            &self.filename
        };
        pattern
            .replace("{date}", &now.format("%Y-%m-%d").to_string())
            .replace("{time}", &now.format("%H.%M.%S").to_string())
            .replace(['/', ':'], "-")
    }
}

/// Copies `old` to `new` unless `new` exists already. True if it copied.
fn migrate_file(old: &Path, new: &Path) -> Result<bool> {
    if new.exists() || !old.is_file() {
        return Ok(false);
    }
    if let Some(dir) = new.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::copy(old, new).with_context(|| format!("copying {}", old.display()))?;
    Ok(true)
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
        let dir = std::env::temp_dir().join(format!("shotly-cfg-{}", std::process::id()));
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
    fn recording_settings_default_and_are_sanitized() {
        let c: Config = toml::from_str("format = \"jpeg\"").unwrap();
        assert_eq!(c.record_hotkey, "Cmd+Shift+1");
        assert_eq!(c.record_fps, 30);
        assert!(c.record_retina && c.highlight_cursor && c.show_clicks);
        assert!(!c.record_mic);
        let c: Config = toml::from_str(
            "record_fps = 24\nhighlight_color = 99\nleft_click_color = 8\nhighlight_size = \"large\"",
        )
        .unwrap();
        let c = c.sanitized();
        assert_eq!(c.record_fps, 30);
        assert_eq!(c.highlight_color, 2);
        assert_eq!(c.left_click_color, 0);
        assert_eq!(c.highlight_size, HighlightSize::Large);
        assert!(HighlightSize::Large.radius_pt() > HighlightSize::Small.radius_pt());
        assert_eq!(Config::default().right_click_color, 4);
    }

    #[test]
    fn preset_is_remembered_by_name_and_has_keys() {
        let c: Config = toml::from_str("preset = \"window\"").unwrap();
        assert_eq!(c.preset, Preset::Window);
        assert_eq!(Config::default().preset, Preset::Custom);
        let text = toml::to_string_pretty(&Config::default()).unwrap();
        assert!(text.contains("preset = \"custom\""), "{text}");
        for p in Preset::ALL {
            assert_eq!(Preset::from_key(p.key()), Some(p));
            assert_eq!(Preset::from_key(p.key().to_ascii_lowercase()), Some(p));
        }
        assert_eq!(Preset::from_key('x'), None);
    }

    #[test]
    fn recording_file_name_is_mp4() {
        let now = chrono::Local
            .with_ymd_and_hms(2026, 11, 10, 9, 5, 7)
            .unwrap();
        assert_eq!(
            Config::default().video_name(now),
            "Screenshot 2026-11-10 at 09.05.07.mp4"
        );
    }

    #[test]
    fn one_bad_value_does_not_reset_the_other_settings() {
        let c = Config::parse_lenient(
            "format = \"jpeg\"\nhighlight_size = \"huge\"\nrecord_fps = 60\nsave_dir = \"~/Shots\"",
        )
        .unwrap();
        assert_eq!(c.format, Format::Jpeg);
        assert_eq!(c.record_fps, 60);
        assert_eq!(c.save_dir, "~/Shots");
        assert_eq!(
            c.highlight_size,
            HighlightSize::Medium,
            "bad value → default"
        );
        // Not TOML at all: still an error (the caller falls back to defaults).
        assert!(Config::parse_lenient("format = ").is_err());
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
    fn old_settings_are_copied_once_and_never_overwrite() {
        let dir = std::env::temp_dir().join(format!("shotly-migrate-{}", std::process::id()));
        let (old, new) = (dir.join("old/config.toml"), dir.join("new/config.toml"));
        std::fs::create_dir_all(old.parent().unwrap()).unwrap();
        std::fs::write(&old, "format = \"jpeg\"").unwrap();
        assert!(migrate_file(&old, &new).unwrap());
        assert_eq!(Config::load_from(&new).format, Format::Jpeg);
        // A later change to the new file is kept.
        std::fs::write(&new, "format = \"png\"").unwrap();
        assert!(!migrate_file(&old, &new).unwrap());
        assert_eq!(Config::load_from(&new).format, Format::Png);
        // Nothing to copy.
        assert!(!migrate_file(&dir.join("none.toml"), &dir.join("x/y.toml")).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
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
