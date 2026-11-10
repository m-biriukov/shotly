//! Tools, colours and stroke widths offered by the toolbar.

use crate::shape::Kind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Rect,
    Ellipse,
    Line,
    Arrow,
    Pen,
    Highlighter,
    Text,
    Blur,
}

impl Tool {
    /// Shape this tool draws; `None` for the select tool.
    pub fn kind(self) -> Option<Kind> {
        match self {
            Self::Select => None,
            Self::Rect => Some(Kind::Rect),
            Self::Ellipse => Some(Kind::Ellipse),
            Self::Line => Some(Kind::Line),
            Self::Arrow => Some(Kind::Arrow),
            Self::Pen => Some(Kind::Pen),
            Self::Highlighter => Some(Kind::Highlighter),
            Self::Text => Some(Kind::Text),
            Self::Blur => Some(Kind::Pixelate),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Select => "Select / move",
            Self::Rect => "Rectangle",
            Self::Ellipse => "Ellipse",
            Self::Line => "Line",
            Self::Arrow => "Arrow",
            Self::Pen => "Pen",
            Self::Highlighter => "Highlighter",
            Self::Text => "Text",
            Self::Blur => "Blur (pixelate)",
        }
    }

    /// The keyboard shortcut that picks this tool.
    pub fn key(self) -> char {
        match self {
            Self::Select => 'V',
            Self::Rect => 'R',
            Self::Ellipse => 'O',
            Self::Line => 'L',
            Self::Arrow => 'A',
            Self::Pen => 'P',
            Self::Highlighter => 'H',
            Self::Text => 'T',
            Self::Blur => 'B',
        }
    }

    /// The tool a lower-case shortcut letter picks (see `keys::latin_shortcut`).
    pub fn from_key(c: char) -> Option<Self> {
        Some(match c.to_ascii_lowercase() {
            'v' => Self::Select,
            'r' => Self::Rect,
            'o' => Self::Ellipse,
            'l' => Self::Line,
            'a' => Self::Arrow,
            'p' => Self::Pen,
            'h' => Self::Highlighter,
            't' => Self::Text,
            'b' => Self::Blur,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    S,
    M,
    L,
}

impl Width {
    pub const ALL: [Self; 3] = [Self::S, Self::M, Self::L];

    /// Size in points of what this width means for `kind`: stroke width,
    /// highlighter width, font size or pixelation block size.
    pub fn pt(self, kind: Kind) -> f32 {
        let (s, m, l) = match kind {
            Kind::Rect | Kind::Ellipse | Kind::Line | Kind::Arrow | Kind::Pen => (2.0, 4.0, 8.0),
            Kind::Highlighter => (10.0, 18.0, 28.0),
            Kind::Text => (14.0, 20.0, 32.0),
            // Blocks smaller than ~10 pt can leave text recoverable
            // (e.g. with Depix), so even "small" stays coarse.
            Kind::Pixelate => (10.0, 16.0, 24.0),
        };
        match self {
            Self::S => s,
            Self::M => m,
            Self::L => l,
        }
    }
}

pub const COLOR_NAMES: [&str; 8] = [
    "Red", "Orange", "Yellow", "Green", "Blue", "Purple", "Black", "White",
];

/// Preset colours as 0xRRGGBB.
pub const PALETTE: [u32; 8] = [
    0xFF3B30, // red
    0xFF9500, // orange
    0xFFCC00, // yellow
    0x34C759, // green
    0x007AFF, // blue
    0xAF52DE, // purple
    0x000000, // black
    0xFFFFFF, // white
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_key_round_trips() {
        for t in [
            Tool::Select,
            Tool::Rect,
            Tool::Ellipse,
            Tool::Line,
            Tool::Arrow,
            Tool::Pen,
            Tool::Highlighter,
            Tool::Text,
            Tool::Blur,
        ] {
            assert_eq!(Tool::from_key(t.key()), Some(t), "{t:?}");
        }
        assert_eq!(COLOR_NAMES.len(), PALETTE.len());
    }

    #[test]
    fn pixelate_blocks_are_never_fine_enough_to_read_text() {
        for w in Width::ALL {
            assert!(w.pt(Kind::Pixelate) >= 10.0, "{w:?}");
        }
    }
}
