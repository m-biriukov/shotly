//! The recognized words of the picture and the selection over them: what the
//! Select-text tool copies and the Text-highlight tool marks. Pure logic, in
//! window pixels.

use crate::geom::{Point, Rect};
use crate::ocr::Word;

/// How far from a word (px at scale 1) a press still lands on it.
pub const SLOP_PT: f64 = 3.0;

/// How far the recognizer has come for this picture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Reading {
    #[default]
    NotAsked,
    Pending,
    /// Pending for a while: the models are still loading.
    Slow,
    Ready,
    Failed,
}

#[derive(Default)]
pub struct TextLayer {
    reading: Reading,
    /// In reading order.
    words: Vec<Word>,
    /// Indices of the word the selection started on and the one it reaches.
    ends: Option<(usize, usize)>,
}

impl TextLayer {
    pub fn reading(&self) -> Reading {
        self.reading
    }

    pub fn set_pending(&mut self) {
        self.reading = Reading::Pending;
    }

    pub fn set_slow(&mut self) {
        if self.reading == Reading::Pending {
            self.reading = Reading::Slow;
        }
    }

    pub fn set_words(&mut self, words: Vec<Word>) {
        self.reading = Reading::Ready;
        self.words = words;
        self.ends = None;
    }

    pub fn set_failed(&mut self) {
        self.reading = Reading::Failed;
    }

    /// Whether any recognized word lies in `area`.
    pub fn has_words_in(&self, area: Rect) -> bool {
        self.words.iter().any(|w| centre_in(w.rect, area))
    }

    /// The word under `p` (within `slop` px), among those inside `area`.
    fn word_at(&self, p: Point, slop: i32, area: Rect) -> Option<usize> {
        self.words
            .iter()
            .position(|w| centre_in(w.rect, area) && w.rect.expand(slop).contains(p))
    }

    /// The word in `area` closest to `p`, preferring the line `p` is level
    /// with, so a drag past the end of a line takes the whole line.
    fn nearest(&self, p: Point, area: Rect) -> Option<usize> {
        self.words
            .iter()
            .enumerate()
            .filter(|(_, w)| centre_in(w.rect, area))
            .min_by_key(|(_, w)| {
                let r = w.rect;
                let dx = i64::from((r.x - p.x).max(p.x - r.right()).max(0));
                let dy = i64::from((r.y - p.y).max(p.y - r.bottom()).max(0));
                // Vertical distance counts for much more than horizontal.
                (dy, dx)
            })
            .map(|(i, _)| i)
    }

    /// A press: starts a selection on the word under `p`, or clears it.
    /// Returns whether a word was hit.
    pub fn press(&mut self, p: Point, slop: i32, area: Rect) -> bool {
        self.ends = self.word_at(p, slop, area).map(|i| (i, i));
        self.ends.is_some()
    }

    /// Extends the selection towards `p`. Returns whether it changed.
    pub fn drag(&mut self, p: Point, area: Rect) -> bool {
        let Some((from, to)) = self.ends else {
            return false;
        };
        let Some(next) = self.nearest(p, area) else {
            return false;
        };
        self.ends = Some((from, next));
        next != to
    }

    pub fn select_all(&mut self, area: Rect) {
        let inside: Vec<usize> = (0..self.words.len())
            .filter(|&i| centre_in(self.words[i].rect, area))
            .collect();
        self.ends = inside.first().copied().zip(inside.last().copied());
    }

    pub fn clear(&mut self) {
        self.ends = None;
    }

    /// The selected words in reading order, those outside `area` left out.
    fn selected(&self, area: Rect) -> impl Iterator<Item = &Word> {
        let (a, b) = self.ends.unwrap_or((1, 0));
        let range = if self.ends.is_some() {
            a.min(b)..a.max(b) + 1
        } else {
            0..0
        };
        self.words[range]
            .iter()
            .filter(move |w| centre_in(w.rect, area))
    }

    /// The selection as text: words joined by spaces, lines by newlines.
    /// `None` when nothing is selected.
    pub fn selected_text(&self, area: Rect) -> Option<String> {
        let mut out = String::new();
        let mut line = None;
        for w in self.selected(area) {
            match line {
                None => {}
                Some(l) if l == w.line => out.push(' '),
                Some(_) => out.push('\n'),
            }
            out.push_str(&w.text);
            line = Some(w.line);
        }
        (!out.is_empty()).then_some(out)
    }

    pub fn selected_count(&self, area: Rect) -> usize {
        self.selected(area).count()
    }

    /// One box per line of the selection, covering its selected words.
    pub fn selected_rects(&self, area: Rect) -> Vec<Rect> {
        let mut rects: Vec<(usize, Rect)> = Vec::new();
        for w in self.selected(area) {
            match rects.last_mut() {
                Some((line, r)) if *line == w.line => *r = r.union(&w.rect),
                _ => rects.push((w.line, w.rect)),
            }
        }
        rects.into_iter().map(|(_, r)| r).collect()
    }
}

fn centre_in(word: Rect, area: Rect) -> bool {
    area.contains(Point::new(word.x + word.w / 2, word.y + word.h / 2))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: Rect = Rect {
        x: 0,
        y: 0,
        w: 1000,
        h: 1000,
    };

    fn word(text: &str, x: i32, y: i32, line: usize) -> Word {
        Word {
            text: text.into(),
            rect: Rect::new(x, y, 40, 10),
            line,
        }
    }

    /// "one two" over "three four".
    fn layer() -> TextLayer {
        let mut t = TextLayer::default();
        t.set_words(vec![
            word("one", 10, 10, 0),
            word("two", 60, 10, 0),
            word("three", 10, 30, 1),
            word("four", 60, 30, 1),
        ]);
        t
    }

    #[test]
    fn reading_states() {
        let mut t = TextLayer::default();
        assert_eq!(t.reading(), Reading::NotAsked);
        t.set_slow();
        assert_eq!(t.reading(), Reading::NotAsked, "only a pending one is slow");
        t.set_pending();
        assert_eq!(t.reading(), Reading::Pending);
        t.set_slow();
        assert_eq!(t.reading(), Reading::Slow);
        t.set_failed();
        assert_eq!(t.reading(), Reading::Failed);
        t.set_words(Vec::new());
        assert_eq!(t.reading(), Reading::Ready);
        assert!(!t.has_words_in(ALL));
    }

    #[test]
    fn a_drag_selects_a_run_of_words_in_reading_order() {
        let mut t = layer();
        assert!(t.press(Point::new(70, 15), 3, ALL), "on 'two'");
        assert_eq!(t.selected_text(ALL).as_deref(), Some("two"));
        assert!(t.drag(Point::new(20, 35), ALL));
        assert_eq!(t.selected_text(ALL).as_deref(), Some("two\nthree"));
        assert_eq!(t.selected_count(ALL), 2);
        // Dragging backwards selects the same way.
        t.press(Point::new(20, 35), 3, ALL);
        t.drag(Point::new(20, 12), ALL);
        assert_eq!(t.selected_text(ALL).as_deref(), Some("one two\nthree"));
    }

    #[test]
    fn dragging_past_the_end_of_a_line_takes_the_line() {
        let mut t = layer();
        t.press(Point::new(15, 15), 3, ALL);
        t.drag(Point::new(900, 14), ALL);
        assert_eq!(t.selected_text(ALL).as_deref(), Some("one two"));
    }

    #[test]
    fn a_press_beside_the_words_clears_the_selection() {
        let mut t = layer();
        t.press(Point::new(15, 15), 3, ALL);
        assert!(!t.press(Point::new(500, 500), 3, ALL));
        assert_eq!(t.selected_text(ALL), None);
        assert!(!t.drag(Point::new(15, 15), ALL), "nothing to extend");
    }

    #[test]
    fn only_words_inside_the_area_count() {
        let mut t = layer();
        let left = Rect::new(0, 0, 55, 1000);
        t.select_all(left);
        assert_eq!(t.selected_text(left).as_deref(), Some("one\nthree"));
        assert!(!t.press(Point::new(70, 15), 3, left), "'two' is outside");
        t.select_all(ALL);
        assert_eq!(t.selected_count(ALL), 4);
        t.clear();
        assert_eq!(t.selected_count(ALL), 0);
    }

    #[test]
    fn highlight_boxes_cover_one_line_each() {
        let mut t = layer();
        t.select_all(ALL);
        assert_eq!(
            t.selected_rects(ALL),
            vec![Rect::new(10, 10, 90, 10), Rect::new(10, 30, 90, 10)]
        );
    }
}
