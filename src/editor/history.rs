//! Undo/redo as whole-state snapshots.

pub struct History<T> {
    undo: Vec<T>,
    redo: Vec<T>,
}

impl<T> Default for History<T> {
    fn default() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }
}

impl<T> History<T> {
    /// Remembers `before` as the state to return to; a new change makes the
    /// redo list obsolete.
    pub fn record(&mut self, before: T) {
        self.undo.push(before);
        self.redo.clear();
    }

    /// Steps back: `current` becomes the previous state. False if none.
    pub fn undo(&mut self, current: &mut T) -> bool {
        let Some(prev) = self.undo.pop() else {
            return false;
        };
        self.redo.push(std::mem::replace(current, prev));
        true
    }

    /// Steps forward again after `undo`. False if nothing to redo.
    pub fn redo(&mut self, current: &mut T) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(current, next));
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_redo_and_new_change_clears_redo() {
        let mut h = History::default();
        let mut state = 1;
        h.record(state);
        state = 2;
        h.record(state);
        state = 3;
        assert!(h.undo(&mut state));
        assert_eq!(state, 2);
        assert!(h.undo(&mut state));
        assert_eq!(state, 1);
        assert!(!h.undo(&mut state));
        assert!(h.redo(&mut state));
        assert_eq!(state, 2);
        h.record(state);
        state = 9;
        assert!(!h.can_redo(), "a new change drops the redo list");
        assert!(h.undo(&mut state));
        assert_eq!(state, 2);
    }
}
