//! Typing into a text shape: caret movement and editing keys.

use crate::shape::Shape;

/// Keys that mean something while typing text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKey {
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    Newline,
    Finish,
}

/// A text shape being typed into. Committed to the editor when finished.
pub struct TextEdit {
    /// Index of the existing shape being edited, or `None` for a new one.
    pub idx: Option<usize>,
    pub shape: Shape,
    /// Caret position in chars.
    caret: usize,
}

impl TextEdit {
    /// Starts editing `shape` with the caret at the end of its text.
    pub fn new(idx: Option<usize>, shape: Shape) -> Self {
        let caret = shape.text.chars().count();
        Self { idx, shape, caret }
    }

    /// Inserts typed characters at the caret (control characters ignored).
    pub fn insert(&mut self, s: &str) {
        for c in s.chars().filter(|c| !c.is_control()) {
            let at = byte_index(&self.shape.text, self.caret);
            self.shape.text.insert(at, c);
            self.caret += 1;
        }
    }

    /// Applies an editing key. `Finish` is the editor's job and ignored here.
    pub fn key(&mut self, key: EditKey) {
        let text = &mut self.shape.text;
        let len = text.chars().count();
        match key {
            EditKey::Backspace if self.caret > 0 => {
                self.caret -= 1;
                text.remove(byte_index(text, self.caret));
            }
            EditKey::Delete if self.caret < len => {
                text.remove(byte_index(text, self.caret));
            }
            EditKey::Left => self.caret = self.caret.saturating_sub(1),
            EditKey::Right => self.caret = (self.caret + 1).min(len),
            EditKey::Home => {
                let before: Vec<char> = text.chars().take(self.caret).collect();
                self.caret = before.iter().rposition(|&c| c == '\n').map_or(0, |i| i + 1);
            }
            EditKey::End => {
                let after = text.chars().skip(self.caret).position(|c| c == '\n');
                self.caret = after.map_or(len, |n| self.caret + n);
            }
            EditKey::Newline => {
                text.insert(byte_index(text, self.caret), '\n');
                self.caret += 1;
            }
            _ => {}
        }
    }

    /// Caret line as (x, top, height) in window pixels.
    pub fn caret_line(&self) -> (f32, f32, f32) {
        let l = self.shape.text_layout();
        let (cx, cy) = l.caret(self.caret);
        let (px, py) = self.shape.text_padding();
        let (ox, oy) = self.shape.anchor().to_f32();
        (ox + px + cx, oy + py + cy, l.line_h)
    }
}

fn byte_index(s: &str, char_idx: usize) -> usize {
    s.char_indices().nth(char_idx).map_or(s.len(), |(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Point;
    use crate::shape::{Kind, Style};

    #[test]
    fn edits_multibyte_text_by_characters() {
        let style = Style {
            color: 0,
            size: 20.0,
            bg: false,
        };
        let mut e = TextEdit::new(None, Shape::new(Kind::Text, Point::new(0, 0), style));
        e.insert("Привет");
        e.key(EditKey::Left);
        e.key(EditKey::Backspace);
        e.insert("\tе");
        e.key(EditKey::End);
        e.key(EditKey::Delete);
        assert_eq!(e.shape.text, "Привет");
        e.key(EditKey::Home);
        e.insert(">");
        assert_eq!(e.shape.text, ">Привет");
    }
}
