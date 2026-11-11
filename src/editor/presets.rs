//! Presets in the picker: Full screen, Window, Custom, and the strip of
//! buttons at the top that switches between them.

use super::{Editor, Preset, Toolbar, toolbar};
use crate::geom::Point;

impl Editor {
    pub fn preset(&self) -> Preset {
        self.preset
    }

    /// Remembers `preset` without selecting anything (window snapping only
    /// works under [`Preset::Window`]).
    pub fn set_preset(&mut self, preset: Preset) {
        self.preset = preset;
    }

    /// Switches to `preset` and does what it means: the whole display is
    /// selected, or the selection is dropped so a window can be clicked.
    pub fn choose_preset(&mut self, preset: Preset) {
        self.finish_edit();
        self.preset = preset;
        self.snap.on_press = None;
        match preset {
            Preset::Full => {
                self.area.set_rect(self.bounds);
                self.selected = None;
                self.drag = None;
            }
            Preset::Window => self.clear_area(),
            Preset::Custom => {}
        }
    }

    /// The Full screen / Window / Custom strip (hidden while dragging).
    pub fn strip(&self) -> Option<Toolbar> {
        (!self.dragging_area()).then(|| toolbar::strip(self.bounds, self.scale))
    }

    /// Strip, flyout, toolbar: the bar under `p`, if any.
    pub(super) fn bar_at(&self, p: Point) -> Option<Toolbar> {
        [self.strip(), self.flyout(), self.toolbar()]
            .into_iter()
            .flatten()
            .find(|b| b.rect.contains(p))
    }
}
