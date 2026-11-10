//! Colours of the app's own UI (toolbar, labels, selection, toasts), so the
//! look is defined in one place. Annotation colours live in `style::PALETTE`.

/// Selection border and handles.
pub const ACCENT: u32 = 0x3B82F6;
/// Toolbar, flyouts, size label and tooltips.
pub const PANEL: u32 = 0x1E1E20;
/// Opacity of `PANEL` over the screen.
pub const PANEL_ALPHA: u8 = 235;
/// Icons and text on `PANEL`.
pub const INK: u32 = 0xF2F2F2;
/// Hover/active highlight behind a toolbar button (drawn with low alpha).
pub const HIGHLIGHT: u32 = 0xFFFFFF;
/// Outline that keeps black and white colour swatches visible.
pub const SWATCH_OUTLINE: u32 = 0x808080;
/// Fill of selection handles.
pub const HANDLE_FILL: u32 = 0xFFFFFF;
/// Toast background and its two text colours.
pub const TOAST_BG: u32 = 0x232326;
pub const TOAST_TITLE: u32 = 0xF5F5F7;
pub const TOAST_DETAIL: u32 = 0xA1A1A6;
