//! Everything that happens on one overlay window: the capture area, the
//! annotation shapes, tool/colour/size choice, text editing, the toolbar and
//! undo/redo. Pure state machine, no windowing, so it can be unit tested.

mod history;
mod presets;
mod text_edit;
mod ui;

use history::History;
pub use text_edit::EditKey;
use text_edit::TextEdit;
use ui::{Snapping, ToolbarUi};

use crate::config::Preset;
use crate::geom::{Handle, Hit, Point, Rect, Scale, Selection};
use crate::shape::{Kind, Shape, ShapeHandle, Style};
use crate::style::{PALETTE, Tool, Width};
use crate::toolbar::{self, Item, Toolbar};

/// What the window should do after an input event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Copy,
    Save,
    /// Start recording the selected area (record mode).
    Record,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cursor {
    Crosshair,
    Move,
    Pointer,
    Text,
    Resize(Handle),
}

/// A hovered button and the toolbar pieces around it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HoverTarget {
    pub item: Item,
    /// The button.
    pub rect: Rect,
    /// The toolbar or flyout holding it.
    pub bar: Rect,
    /// The other one, if a flyout is open.
    pub other: Option<Rect>,
}

#[derive(Clone, Debug)]
enum Drag {
    /// Creating, moving or resizing the capture area.
    Area,
    /// Drawing the draft shape.
    Draw,
    Move {
        idx: usize,
        start: Point,
        orig: Shape,
    },
    Handle {
        idx: usize,
        handle: ShapeHandle,
        start: Point,
        orig: Shape,
    },
    /// Press already handled (toolbar, finishing a text edit); ignore until release.
    Ignore,
}

/// What new shapes look like: the toolbar's current choices.
struct Choices {
    tool: Tool,
    /// Palette index.
    color: usize,
    width: Width,
    text_bg: bool,
}

/// Handle grab distance and shape hit tolerance, in points.
const GRAB_PT: f64 = 8.0;
const HIT_PT: f32 = 5.0;

pub struct Editor {
    bounds: Rect,
    scale: Scale,
    area: Selection,
    shapes: Vec<Shape>,
    selected: Option<usize>,
    draft: Option<Shape>,
    drag: Option<Drag>,
    edit: Option<TextEdit>,
    choices: Choices,
    history: History<Vec<Shape>>,
    ui: ToolbarUi,
    snap: Snapping,
    /// Picking an area to record: no drawing, a Record button instead of Copy/Save.
    record: bool,
    /// How the area is picked: whole display, a window, or by hand.
    preset: Preset,
}

impl Editor {
    /// `bounds` is the window in physical pixels, `scale` its backing scale.
    pub fn new(bounds: Rect, scale: impl Into<Scale>) -> Self {
        let scale = scale.into();
        Self {
            bounds,
            scale,
            area: Selection::new(bounds, scale.px(GRAB_PT)),
            shapes: Vec::new(),
            selected: None,
            draft: None,
            drag: None,
            edit: None,
            choices: Choices {
                tool: Tool::Rect,
                color: 0,
                width: Width::M,
                text_bg: false,
            },
            history: History::default(),
            ui: ToolbarUi::default(),
            snap: Snapping::default(),
            record: false,
            preset: Preset::Custom,
        }
    }

    /// Switches to picking an area to record: the select tool only, and a
    /// toolbar with just Record and Close.
    pub fn set_record_mode(&mut self) {
        self.record = true;
        self.choices.tool = Tool::Select;
    }

    /// Starting colour and size (no shape is touched).
    pub fn set_defaults(&mut self, color: usize, width: Width) {
        self.choices.color = color.min(PALETTE.len() - 1);
        self.choices.width = width;
    }

    /// Window frames available for click-to-select, front to back.
    pub fn set_snaps(&mut self, frames: Vec<Rect>) {
        self.snap.frames = frames;
    }

    pub fn color(&self) -> usize {
        self.choices.color
    }

    pub fn width(&self) -> Width {
        self.choices.width
    }

    /// Highlighted window frame shown before any area is chosen.
    pub fn preview(&self) -> Option<Rect> {
        if self.area().is_none() && self.drag.is_none() && self.preset == Preset::Window {
            self.snap.hover
        } else {
            None
        }
    }

    // ---- read access for rendering/export ----

    pub fn area(&self) -> Option<Rect> {
        self.area.rect().filter(|r| !r.is_empty())
    }

    /// Committed shapes only; call `finish_edit` first when exporting.
    pub fn shapes(&self) -> &[Shape] {
        &self.shapes
    }

    /// Index of the committed shape currently being moved/resized/edited.
    fn busy_index(&self) -> Option<usize> {
        match &self.drag {
            Some(Drag::Move { idx, .. } | Drag::Handle { idx, .. }) => Some(*idx),
            _ => self.edit.as_ref().and_then(|e| e.idx),
        }
    }

    /// Shapes that are not changing right now, in paint order.
    pub fn static_shapes(&self) -> impl Iterator<Item = &Shape> {
        let busy = self.busy_index();
        self.shapes
            .iter()
            .enumerate()
            .filter(move |(i, _)| Some(*i) != busy)
            .map(|(_, s)| s)
    }

    /// The one shape changing right now (being drawn, dragged or typed), so
    /// only it needs repainting on every mouse move.
    pub fn active_shape(&self) -> Option<&Shape> {
        self.draft
            .as_ref()
            .or_else(|| self.edit.as_ref().map(|e| &e.shape))
            .or_else(|| self.busy_index().map(|i| &self.shapes[i]))
    }

    /// Changes whenever `static_shapes` would paint differently.
    pub fn static_key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for s in self.static_shapes() {
            (s.kind as u8).hash(&mut h);
            s.geom.hash(&mut h);
            s.style.color.hash(&mut h);
            s.style.size.to_bits().hash(&mut h);
            s.style.bg.hash(&mut h);
            s.text.hash(&mut h);
        }
        h.finish()
    }

    pub fn selected_shape(&self) -> Option<&Shape> {
        self.selected.map(|i| &self.shapes[i])
    }

    pub fn editing_shape(&self) -> Option<&Shape> {
        self.edit.as_ref().map(|e| &e.shape)
    }

    pub fn is_editing(&self) -> bool {
        self.edit.is_some()
    }

    /// Caret line as (x, top, height) in window pixels while typing.
    pub fn caret(&self) -> Option<(f32, f32, f32)> {
        self.edit.as_ref().map(TextEdit::caret_line)
    }

    #[cfg(test)]
    pub fn tool(&self) -> Tool {
        self.choices.tool
    }

    pub fn dragging_area(&self) -> bool {
        matches!(self.drag, Some(Drag::Area))
    }

    pub fn toolbar(&self) -> Option<Toolbar> {
        if self.dragging_area() {
            return None;
        }
        self.area()
            .map(|a| toolbar::layout(a, self.bounds, self.scale, self.record))
    }

    /// The open group's flyout, if any.
    pub fn flyout(&self) -> Option<Toolbar> {
        let tb = self.toolbar()?;
        toolbar::flyout(&tb, self.ui.open?, self.bounds, self.scale)
    }

    /// Closes an open flyout; returns whether one was open.
    pub fn close_flyout(&mut self) -> bool {
        self.ui.open.take().is_some()
    }

    /// The hovered toolbar/flyout button with the bars around it, for tooltips.
    pub fn hover_target(&self) -> Option<HoverTarget> {
        let item = self.ui.hover?;
        if let Some(strip) = self.strip()
            && let Some(&(_, rect)) = strip.items.iter().find(|(i, _)| *i == item)
        {
            return Some(HoverTarget {
                item,
                rect,
                bar: strip.rect,
                other: None,
            });
        }
        let tb = self.toolbar()?;
        let fly = self.flyout();
        if let Some(f) = &fly
            && let Some(&(_, rect)) = f.items.iter().find(|(i, _)| *i == item)
        {
            return Some(HoverTarget {
                item,
                rect,
                bar: f.rect,
                other: Some(tb.rect),
            });
        }
        let &(_, rect) = tb.items.iter().find(|(i, _)| *i == item)?;
        Some(HoverTarget {
            item,
            rect,
            bar: tb.rect,
            other: fly.map(|f| f.rect),
        })
    }

    /// Forgets the hover highlights (cursor left the window); returns
    /// whether anything changed.
    pub fn clear_hover(&mut self) -> bool {
        if self.drag.is_some() {
            return false;
        }
        let changed = self.ui.hover.is_some() || self.snap.hover.is_some();
        self.ui.hover = None;
        self.snap.hover = None;
        changed
    }

    pub fn toolbar_state(&self) -> toolbar::State {
        toolbar::State {
            tool: self.choices.tool,
            color: self.choices.color,
            width: self.choices.width,
            text_bg: self.choices.text_bg,
            preset: self.preset,
            shape_tool: self.ui.shape_tool,
            draw_tool: self.ui.draw_tool,
            open: self.ui.open,
            can_undo: self.history.can_undo() || self.edit.is_some(),
            can_redo: self.history.can_redo(),
            hover: self.ui.hover,
        }
    }

    // ---- window changes ----

    pub fn set_bounds(&mut self, bounds: Rect) {
        self.bounds = bounds;
        self.area.set_bounds(bounds);
    }

    /// Drops the capture area (another display took over). Shapes stay.
    pub fn clear_area(&mut self) {
        self.finish_edit();
        self.area.clear();
        self.drag = None;
        self.draft = None;
        self.selected = None;
    }

    // ---- commands ----

    fn style_for(&self, kind: Kind) -> Style {
        let c = &self.choices;
        Style {
            color: PALETTE[c.color],
            size: self.scale.pxf(c.width.pt(kind)),
            bg: c.text_bg,
        }
    }

    /// Records the current shapes as an undo step.
    fn checkpoint(&mut self) {
        self.history.record(self.shapes.clone());
    }

    /// Applies `f` to the text being typed (no undo step: the whole edit is
    /// one step) or else to the selected shape (one undo step if it changed).
    fn restyle(&mut self, f: impl Fn(&mut Shape)) {
        if let Some(e) = &mut self.edit {
            f(&mut e.shape);
        } else if let Some(idx) = self.selected {
            let mut s = self.shapes[idx].clone();
            f(&mut s);
            if s != self.shapes[idx] {
                self.checkpoint();
                self.shapes[idx] = s;
            }
        }
    }

    pub fn set_tool(&mut self, tool: Tool) {
        self.finish_edit();
        self.choices.tool = tool;
        self.ui.remember(tool);
    }

    /// Picks a colour; also recolours the selected or edited shape.
    pub fn set_color(&mut self, i: usize) {
        self.choices.color = i;
        self.restyle(|s| s.style.color = PALETTE[i]);
    }

    /// Picks a size; also resizes the selected or edited shape.
    pub fn set_width(&mut self, w: Width) {
        self.choices.width = w;
        let scale = self.scale;
        self.restyle(|s| s.style.size = scale.pxf(w.pt(s.kind)));
    }

    /// Toggles the filled box behind text; applies to selected/edited text.
    pub fn toggle_text_bg(&mut self) {
        self.choices.text_bg = !self.choices.text_bg;
        let bg = self.choices.text_bg;
        self.restyle(|s| {
            if s.kind == Kind::Text {
                s.style.bg = bg;
            }
        });
    }

    pub fn undo(&mut self) {
        if self.drag.is_some() {
            return;
        }
        self.finish_edit();
        if self.history.undo(&mut self.shapes) {
            self.selected = None;
        }
    }

    pub fn redo(&mut self) {
        if self.drag.is_some() {
            return;
        }
        self.finish_edit();
        if self.history.redo(&mut self.shapes) {
            self.selected = None;
        }
    }

    pub fn delete_selected(&mut self) {
        if self.drag.is_some() || self.edit.is_some() {
            return;
        }
        if let Some(i) = self.selected.take() {
            self.checkpoint();
            self.shapes.remove(i);
        }
    }

    fn click_item(&mut self, item: Item) -> Option<Action> {
        // Any choice closes the flyout; a group button toggles its own.
        let was_open = self.ui.open.take();
        match item {
            Item::Group(g) => {
                if was_open != Some(g) {
                    self.ui.open = Some(g);
                }
                // Tool groups also switch to their last-used tool right away.
                if let Some(t) = self.ui.tool_for(g) {
                    self.set_tool(t);
                }
            }
            Item::Tool(t) => self.set_tool(t),
            Item::Color(i) => self.set_color(i),
            Item::Width(w) => self.set_width(w),
            Item::TextBg => self.toggle_text_bg(),
            Item::Undo => self.undo(),
            Item::Redo => self.redo(),
            Item::Copy => return Some(Action::Copy),
            Item::Save => return Some(Action::Save),
            Item::Record => return Some(Action::Record),
            Item::Preset(p) => self.choose_preset(p),
            Item::Close => return Some(Action::Cancel),
        }
        None
    }

    // ---- text editing ----

    fn start_edit(&mut self, idx: Option<usize>, at: Point) {
        self.finish_edit();
        let shape = match idx {
            Some(i) => self.shapes[i].clone(),
            None => {
                let mut s = Shape::new(Kind::Text, at, self.style_for(Kind::Text));
                // Put the click in the middle of the first line.
                let (px, py) = s.text_padding();
                let half = s.text_layout().line_h / 2.0;
                s.move_anchor_to(Point::new(
                    at.x - px.round() as i32,
                    at.y - (py + half).round() as i32,
                ));
                s
            }
        };
        self.selected = None;
        self.edit = Some(TextEdit::new(idx, shape));
    }

    /// Commits the text being typed. Empty text is dropped (and an edited
    /// shape that was emptied is deleted).
    pub fn finish_edit(&mut self) {
        let Some(e) = self.edit.take() else { return };
        let empty = e.shape.text.trim().is_empty();
        match e.idx {
            Some(i) if empty => {
                self.checkpoint();
                self.shapes.remove(i);
                self.selected = None;
            }
            Some(i) => {
                if self.shapes[i] != e.shape {
                    self.checkpoint();
                    self.shapes[i] = e.shape;
                }
                self.selected = Some(i);
            }
            None if empty => {}
            None => {
                self.checkpoint();
                self.shapes.push(e.shape);
                self.selected = Some(self.shapes.len() - 1);
            }
        }
    }

    /// Inserts typed characters at the caret (control characters ignored).
    pub fn insert_text(&mut self, s: &str) {
        if let Some(e) = &mut self.edit {
            e.insert(s);
        }
    }

    pub fn edit_key(&mut self, key: EditKey) {
        match (&mut self.edit, key) {
            (_, EditKey::Finish) => self.finish_edit(),
            (Some(e), k) => e.key(k),
            (None, _) => {}
        }
    }

    // ---- hit testing ----

    fn shape_at(&self, p: Point) -> Option<usize> {
        let tol = self.scale.pxf(HIT_PT);
        (0..self.shapes.len())
            .rev()
            .find(|&i| self.shapes[i].hit(p, tol))
    }

    fn text_at(&self, p: Point) -> Option<usize> {
        (0..self.shapes.len())
            .rev()
            .find(|&i| self.shapes[i].kind == Kind::Text && self.shapes[i].hit(p, 0.0))
    }

    fn selected_handle_at(&self, p: Point) -> Option<(usize, ShapeHandle)> {
        let i = self.selected?;
        let g = self.scale.px(GRAB_PT);
        let near = |c: &Point| (p.x - c.x).abs() <= g && (p.y - c.y).abs() <= g;
        self.shapes[i]
            .handles()
            .into_iter()
            .find(|(_, c)| near(c))
            .map(|(h, _)| (i, h))
    }

    // ---- mouse ----

    /// `double` is set for the second press of a double-click.
    pub fn press(&mut self, p: Point, double: bool) -> Option<Action> {
        self.snap.on_press = None;
        if let Some(handled) = self.press_bars(p) {
            self.drag = Some(Drag::Ignore);
            return handled;
        }
        self.press_canvas(p, double);
        None
    }

    /// Presses on the toolbar or a flyout, or that close an open flyout.
    /// `Some(action)` when the press was used up here.
    fn press_bars(&mut self, p: Point) -> Option<Option<Action>> {
        if let Some(strip) = self.strip().filter(|b| b.rect.contains(p)) {
            return Some(strip.hit(p).and_then(|item| self.click_item(item)));
        }
        let on_toolbar = self.toolbar().filter(|tb| tb.rect.contains(p));
        if let Some(f) = self.flyout() {
            if f.rect.contains(p) {
                return Some(f.hit(p).and_then(|item| self.click_item(item)));
            }
            // A click elsewhere on the canvas only closes the flyout.
            if on_toolbar.is_none() {
                self.ui.open = None;
                return Some(None);
            }
        }
        let tb = on_toolbar?;
        let action = tb.hit(p).and_then(|item| self.click_item(item));
        if action.is_some() {
            self.finish_edit();
        }
        Some(action)
    }

    fn press_canvas(&mut self, p: Point, double: bool) {
        if let Some(e) = &self.edit {
            // Clicking inside the text keeps typing; anywhere else finishes it.
            if !e.shape.bbox().contains(p) {
                self.finish_edit();
            }
            self.drag = Some(Drag::Ignore);
            return;
        }
        if self.area().is_none() {
            match self.preset {
                // Only a click picks a window; a drag does nothing.
                Preset::Window => {
                    self.snap.on_press = self.snap.at(p);
                    self.snap.press_at = Some(p);
                    self.drag = Some(Drag::Ignore);
                }
                Preset::Full => {
                    self.choose_preset(Preset::Full);
                    self.drag = Some(Drag::Ignore);
                }
                Preset::Custom => self.press_area(p),
            }
            return;
        }
        // Window: a click outside the window picks another one.
        if self.preset == Preset::Window && matches!(self.area.hit(p), Hit::Outside) {
            self.snap.on_press = self.snap.at(p);
            self.snap.press_at = Some(p);
            self.drag = Some(Drag::Ignore);
            return;
        }
        if let Some((idx, handle)) = self.selected_handle_at(p) {
            let orig = self.shapes[idx].clone();
            self.drag = Some(Drag::Handle {
                idx,
                handle,
                start: p,
                orig,
            });
            return;
        }
        if let Hit::Handle(_) = self.area.hit(p) {
            return self.press_area(p);
        }
        if (double || self.choices.tool == Tool::Text)
            && let Some(idx) = self.text_at(p)
        {
            self.start_edit(Some(idx), p);
            self.drag = Some(Drag::Ignore);
            return;
        }
        match self.choices.tool.kind() {
            None => match self.shape_at(p) {
                Some(idx) => {
                    self.selected = Some(idx);
                    let orig = self.shapes[idx].clone();
                    self.drag = Some(Drag::Move {
                        idx,
                        start: p,
                        orig,
                    });
                }
                None => {
                    self.selected = None;
                    self.press_area(p);
                }
            },
            Some(Kind::Text) => {
                self.start_edit(None, p);
                self.drag = Some(Drag::Ignore);
            }
            Some(kind) => {
                self.draft = Some(Shape::new(kind, p, self.style_for(kind)));
                self.drag = Some(Drag::Draw);
            }
        }
    }

    fn press_area(&mut self, p: Point) {
        self.area.press(p);
        self.drag = Some(Drag::Area);
    }

    /// Returns true when something visible changed.
    pub fn motion(&mut self, p: Point, shift: bool) -> bool {
        match &self.drag {
            None | Some(Drag::Ignore) => {
                let hover = [self.strip(), self.flyout(), self.toolbar()]
                    .into_iter()
                    .flatten()
                    .find_map(|bar| bar.hit(p));
                let snap = if self.area().is_none()
                    && self.preset == Preset::Window
                    && self.bar_at(p).is_none()
                {
                    self.snap.at(p)
                } else {
                    None
                };
                let changed = hover != self.ui.hover || snap != self.snap.hover;
                self.ui.hover = hover;
                self.snap.hover = snap;
                changed
            }
            Some(Drag::Area) => self.area.motion(p),
            Some(Drag::Draw) => {
                if let Some(d) = &mut self.draft {
                    d.drag_to(p, shift);
                }
                true
            }
            Some(Drag::Move { idx, start, orig }) => {
                let mut s = orig.clone();
                s.translate(p.x - start.x, p.y - start.y);
                self.shapes[*idx] = s;
                true
            }
            Some(Drag::Handle {
                idx,
                handle,
                start,
                orig,
            }) => {
                let (idx, handle, start, orig) = (*idx, *handle, *start, orig.clone());
                self.shapes[idx].drag_handle(&orig, handle, start, p, shift);
                true
            }
        }
    }

    pub fn release(&mut self, p: Point, shift: bool) {
        self.motion(p, shift);
        match self.drag.take() {
            Some(Drag::Ignore) => {
                // A click on a window (Window preset) picks it.
                let click_slop = self.scale.px(GRAB_PT / 2.0);
                let press = self.snap.press_at.take();
                let clicked = press.is_some_and(|q| {
                    (p.x - q.x).abs() <= click_slop && (p.y - q.y).abs() <= click_slop
                });
                let window = self.snap.on_press.take();
                if self.preset == Preset::Window
                    && clicked
                    && let Some(r) = window
                {
                    self.area.set_rect(r);
                }
            }
            Some(Drag::Area) => {
                self.area.release(p);
                // A click (no drag) before any area exists picks the window under it.
                if let Some(r) = self.snap.on_press.take()
                    && self.area().is_none()
                {
                    self.area.set_rect(r);
                }
            }
            Some(Drag::Draw) => {
                let Some(shape) = self.draft.take() else {
                    return;
                };
                if shape.is_degenerate() {
                    // A click: select whatever is under it, or deselect.
                    self.selected = self.shape_at(p);
                } else {
                    self.checkpoint();
                    self.shapes.push(shape);
                    self.selected = Some(self.shapes.len() - 1);
                }
            }
            Some(Drag::Move { idx, orig, .. } | Drag::Handle { idx, orig, .. })
                if self.shapes[idx] != orig =>
            {
                let mut before = self.shapes.clone();
                before[idx] = orig;
                self.history.record(before);
            }
            // Nothing was dragged, or nothing moved.
            None | Some(Drag::Move { .. } | Drag::Handle { .. }) => {}
        }
    }

    pub fn cursor(&self, p: Point) -> Cursor {
        let area_cursor = |hit| match hit {
            Hit::Handle(h) => Cursor::Resize(h),
            Hit::Inside => Cursor::Move,
            Hit::Outside => Cursor::Crosshair,
        };
        match &self.drag {
            Some(Drag::Move { .. }) => return Cursor::Move,
            Some(Drag::Handle {
                handle: ShapeHandle::Box(h),
                ..
            }) => return Cursor::Resize(*h),
            // Drawing, or dragging a line/arrow endpoint.
            Some(Drag::Draw | Drag::Handle { .. }) => return Cursor::Crosshair,
            Some(Drag::Area) => return area_cursor(self.area.hit(p)),
            Some(Drag::Ignore) | None => {}
        }
        if self.bar_at(p).is_some() {
            return Cursor::Pointer;
        }
        if let Some(e) = &self.edit {
            return if e.shape.bbox().contains(p) {
                Cursor::Text
            } else {
                Cursor::Pointer
            };
        }
        if self.area().is_none() {
            return Cursor::Crosshair;
        }
        if let Some((_, h)) = self.selected_handle_at(p) {
            return match h {
                ShapeHandle::Box(h) => Cursor::Resize(h),
                _ => Cursor::Crosshair,
            };
        }
        let hit = self.area.hit(p);
        if let Hit::Handle(h) = hit {
            return Cursor::Resize(h);
        }
        match self.choices.tool {
            Tool::Select if self.shape_at(p).is_some() => Cursor::Move,
            Tool::Select => area_cursor(hit),
            Tool::Text => Cursor::Text,
            _ => Cursor::Crosshair,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolbar::Group;

    fn editor_with_area() -> Editor {
        let mut e = Editor::new(Rect::new(0, 0, 2000, 1500), 1.0);
        drag(&mut e, (100, 100), (900, 700));
        e
    }

    fn drag(e: &mut Editor, from: (i32, i32), to: (i32, i32)) {
        e.press(Point::new(from.0, from.1), false);
        e.motion(Point::new(to.0, to.1), false);
        e.release(Point::new(to.0, to.1), false);
    }

    fn click(e: &mut Editor, at: (i32, i32), double: bool) {
        e.press(Point::new(at.0, at.1), double);
        e.release(Point::new(at.0, at.1), false);
    }

    #[test]
    fn first_drag_makes_area_and_shows_toolbar() {
        let e = editor_with_area();
        assert_eq!(e.area(), Some(Rect::new(100, 100, 800, 600)));
        assert!(e.toolbar().is_some());
        assert!(e.shapes().is_empty());
    }

    #[test]
    fn drawing_adds_selected_shape_with_undo_redo() {
        let mut e = editor_with_area();
        e.set_tool(Tool::Rect);
        drag(&mut e, (200, 200), (300, 300));
        assert_eq!(e.shapes().len(), 1);
        assert_eq!(e.shapes()[0].kind, Kind::Rect);
        assert!(e.selected_shape().is_some());
        e.undo();
        assert!(e.shapes().is_empty());
        e.redo();
        assert_eq!(e.shapes().len(), 1);
    }

    #[test]
    fn click_with_draw_tool_adds_nothing() {
        let mut e = editor_with_area();
        e.set_tool(Tool::Arrow);
        drag(&mut e, (200, 200), (201, 200));
        assert!(e.shapes().is_empty());
        assert!(!e.toolbar_state().can_undo);
    }

    #[test]
    fn select_tool_moves_shape_and_undo_restores() {
        let mut e = editor_with_area();
        e.set_tool(Tool::Line);
        drag(&mut e, (200, 200), (400, 200));
        e.set_tool(Tool::Select);
        drag(&mut e, (300, 201), (300, 251));
        assert_eq!(
            e.shapes()[0].geom.points(),
            vec![Point::new(200, 250), Point::new(400, 250)]
        );
        e.undo();
        assert_eq!(
            e.shapes()[0].geom.points(),
            vec![Point::new(200, 200), Point::new(400, 200)]
        );
    }

    #[test]
    fn select_tool_on_empty_area_moves_area() {
        let mut e = editor_with_area();
        e.set_tool(Tool::Select);
        drag(&mut e, (500, 400), (550, 450));
        assert_eq!(e.area(), Some(Rect::new(150, 150, 800, 600)));
    }

    #[test]
    fn selected_shape_handle_resizes() {
        let mut e = editor_with_area();
        e.set_tool(Tool::Rect);
        drag(&mut e, (200, 200), (300, 300));
        drag(&mut e, (300, 300), (350, 400));
        assert_eq!(e.shapes()[0].bbox(), Rect::new(200, 200, 150, 200));
        assert_eq!(e.shapes().len(), 1);
    }

    #[test]
    fn delete_and_recolor_selected() {
        let mut e = editor_with_area();
        e.set_tool(Tool::Ellipse);
        drag(&mut e, (200, 200), (300, 300));
        e.set_color(4);
        assert_eq!(e.shapes()[0].style.color, PALETTE[4]);
        e.set_width(Width::L);
        assert_eq!(e.shapes()[0].style.size, 8.0);
        e.delete_selected();
        assert!(e.shapes().is_empty());
        e.undo();
        e.undo();
        assert_eq!(e.shapes()[0].style.color, PALETTE[4]);
        assert_eq!(e.shapes()[0].style.size, 4.0);
    }

    fn center_of(tb: &Toolbar, item: Item) -> Point {
        let r = tb.items.iter().find(|(i, _)| *i == item).unwrap().1;
        Point::new(r.x + r.w / 2, r.y + r.h / 2)
    }

    #[test]
    fn group_button_picks_last_tool_and_flyout_switches_it() {
        let mut e = editor_with_area();
        let tb = e.toolbar().unwrap();
        click(
            &mut e,
            (
                center_of(&tb, Item::Group(Group::Shapes)).x,
                center_of(&tb, Item::Group(Group::Shapes)).y,
            ),
            false,
        );
        assert_eq!(e.tool(), Tool::Rect);
        let f = e.flyout().expect("flyout open");
        assert!(
            f.items.iter().all(|(i, _)| *i != Item::Tool(Tool::Arrow)),
            "arrow is not in the flyout"
        );
        let ellipse = center_of(&f, Item::Tool(Tool::Ellipse));
        click(&mut e, (ellipse.x, ellipse.y), false);
        assert_eq!(e.tool(), Tool::Ellipse);
        assert!(e.flyout().is_none(), "choice closes flyout");
        assert_eq!(e.toolbar_state().shape_tool, Tool::Ellipse);
        // Arrow is its own button and leaves the Shapes button alone.
        let tb = e.toolbar().unwrap();
        let a = center_of(&tb, Item::Tool(Tool::Arrow));
        click(&mut e, (a.x, a.y), false);
        assert_eq!(e.tool(), Tool::Arrow);
        assert_eq!(e.toolbar_state().shape_tool, Tool::Ellipse);
        assert!(e.flyout().is_none());
        // Keyboard choices update the group button too.
        e.set_tool(Tool::Highlighter);
        assert_eq!(e.toolbar_state().draw_tool, Tool::Highlighter);
    }

    #[test]
    fn color_flyout_and_click_outside_closes_without_drawing() {
        let mut e = editor_with_area();
        e.set_tool(Tool::Rect);
        let tb = e.toolbar().unwrap();
        let c = center_of(&tb, Item::Group(Group::Color));
        click(&mut e, (c.x, c.y), false);
        assert_eq!(e.tool(), Tool::Rect, "colour group does not change tool");
        let green = center_of(&e.flyout().unwrap(), Item::Color(3));
        click(&mut e, (green.x, green.y), false);
        assert_eq!(e.toolbar_state().color, 3);

        click(&mut e, (c.x, c.y), false);
        assert!(e.flyout().is_some());
        drag(&mut e, (300, 300), (400, 400));
        assert!(e.flyout().is_none());
        assert!(e.shapes().is_empty(), "first click only closes the flyout");
        // Clicking the open group's button again closes it.
        click(&mut e, (c.x, c.y), false);
        click(&mut e, (c.x, c.y), false);
        assert!(e.flyout().is_none());
    }

    #[test]
    fn undo_and_redo_buttons_work() {
        let mut e = editor_with_area();
        e.set_tool(Tool::Rect);
        drag(&mut e, (200, 200), (300, 300));
        assert_eq!(e.shapes().len(), 1);
        let tb = e.toolbar().unwrap();
        let undo = center_of(&tb, Item::Undo);
        click(&mut e, (undo.x, undo.y), false);
        assert!(e.shapes().is_empty(), "undo button");
        let tb = e.toolbar().unwrap();
        let redo = center_of(&tb, Item::Redo);
        click(&mut e, (redo.x, redo.y), false);
        assert_eq!(e.shapes().len(), 1, "redo button");
    }

    #[test]
    fn rectangle_is_the_default_tool() {
        let e = Editor::new(Rect::new(0, 0, 100, 100), 1.0);
        assert_eq!(e.tool(), Tool::Rect);
        assert_eq!(e.toolbar_state().shape_tool, Tool::Rect);
    }

    #[test]
    fn toolbar_clicks_switch_tool_and_return_actions() {
        let mut e = editor_with_area();
        let tb = e.toolbar().unwrap();
        let center = |item: Item| center_of(&tb, item);
        assert_eq!(e.press(center(Item::Tool(Tool::Text)), false), None);
        e.release(center(Item::Tool(Tool::Text)), false);
        assert_eq!(e.tool(), Tool::Text);
        assert!(e.shapes().is_empty());
        assert_eq!(e.press(center(Item::Copy), false), Some(Action::Copy));
        e.release(center(Item::Copy), false);
        assert_eq!(e.press(center(Item::Close), false), Some(Action::Cancel));
    }

    #[test]
    fn shift_constrains_while_drawing() {
        let mut e = editor_with_area();
        e.set_tool(Tool::Arrow);
        e.press(Point::new(200, 200), false);
        e.motion(Point::new(400, 210), true);
        e.release(Point::new(400, 210), true);
        assert_eq!(e.shapes()[0].geom.points()[1], Point::new(400, 200));
    }

    #[test]
    fn window_preset_picks_a_window_by_click_and_ignores_drags() {
        let mut e = Editor::new(Rect::new(0, 0, 2000, 1500), 1.0);
        e.set_snaps(vec![
            Rect::new(100, 100, 300, 200),
            Rect::new(50, 50, 1000, 1000),
        ]);
        e.choose_preset(Preset::Window);
        e.motion(Point::new(150, 150), false);
        assert_eq!(e.preview(), Some(Rect::new(100, 100, 300, 200)));
        e.motion(Point::new(600, 600), false);
        assert_eq!(e.preview(), Some(Rect::new(50, 50, 1000, 1000)));
        // A drag is not a selection.
        drag(&mut e, (150, 150), (200, 220));
        assert_eq!(e.area(), None);
        click(&mut e, (150, 150), false);
        assert_eq!(e.area(), Some(Rect::new(100, 100, 300, 200)));
        assert_eq!(e.preview(), None);
        // A click outside it picks another window.
        click(&mut e, (600, 600), false);
        assert_eq!(e.area(), Some(Rect::new(50, 50, 1000, 1000)));
    }

    #[test]
    fn custom_preset_ignores_windows_and_drags_an_area() {
        let mut e = Editor::new(Rect::new(0, 0, 2000, 1500), 1.0);
        e.set_snaps(vec![Rect::new(100, 100, 300, 200)]);
        e.motion(Point::new(150, 150), false);
        assert_eq!(e.preview(), None);
        drag(&mut e, (150, 150), (200, 220));
        assert_eq!(e.area(), Some(Rect::new(150, 150, 50, 70)));
    }

    #[test]
    fn full_screen_preset_selects_the_whole_display() {
        let mut e = Editor::new(Rect::new(0, 0, 2000, 1500), 1.0);
        e.choose_preset(Preset::Full);
        assert_eq!(e.area(), Some(Rect::new(0, 0, 2000, 1500)));
        assert!(e.toolbar().is_some());
        // Back to Window: the selection is dropped, ready for a click.
        e.choose_preset(Preset::Window);
        assert_eq!(e.area(), None);
    }

    #[test]
    fn strip_buttons_switch_presets() {
        let mut e = Editor::new(Rect::new(0, 0, 2000, 1500), 1.0);
        let strip = e.strip().unwrap();
        let centre = |bar: &Toolbar, item| {
            let r = bar.items.iter().find(|(i, _)| *i == item).unwrap().1;
            (r.x + r.w / 2, r.y + r.h / 2)
        };
        let full = centre(&strip, Item::Preset(Preset::Full));
        click(&mut e, full, false);
        assert_eq!(e.preset(), Preset::Full);
        assert_eq!(e.area(), Some(Rect::new(0, 0, 2000, 1500)));
        let custom = centre(&e.strip().unwrap(), Item::Preset(Preset::Custom));
        click(&mut e, custom, false);
        assert_eq!(e.preset(), Preset::Custom);
        // The area stays; a new drag outside it now starts a new one by hand.
        assert_eq!(e.area(), Some(Rect::new(0, 0, 2000, 1500)));
    }

    #[test]
    fn sizes_depend_on_kind() {
        let mut e = editor_with_area();
        e.set_tool(Tool::Highlighter);
        drag(&mut e, (200, 200), (400, 200));
        assert_eq!(e.shapes()[0].style.size, 18.0);
        e.set_tool(Tool::Blur);
        drag(&mut e, (200, 300), (400, 400));
        assert_eq!(e.shapes()[1].kind, Kind::Pixelate);
        assert_eq!(e.shapes()[1].style.size, 16.0);
    }

    #[test]
    fn typing_text_commits_as_one_undo_step() {
        let mut e = editor_with_area();
        e.set_tool(Tool::Text);
        click(&mut e, (300, 300), false);
        assert!(e.is_editing());
        e.insert_text("Hi");
        e.edit_key(EditKey::Backspace);
        e.insert_text("ey");
        e.edit_key(EditKey::Left);
        e.insert_text("\r!");
        e.set_color(3);
        assert!(e.shapes().is_empty(), "not committed while typing");
        assert_eq!(
            e.static_shapes().count() + e.active_shape().iter().count(),
            1
        );
        e.edit_key(EditKey::Finish);
        assert!(!e.is_editing());
        assert_eq!(e.shapes()[0].text, "He!y");
        assert_eq!(e.shapes()[0].style.color, PALETTE[3]);
        e.undo();
        assert!(e.shapes().is_empty());
    }

    #[test]
    fn empty_text_is_dropped_and_click_away_finishes() {
        let mut e = editor_with_area();
        e.set_tool(Tool::Text);
        click(&mut e, (300, 300), false);
        click(&mut e, (600, 600), false);
        assert!(!e.is_editing());
        assert!(e.shapes().is_empty());
        assert!(!e.toolbar_state().can_undo);
    }

    #[test]
    fn double_click_reedits_text_and_emptying_deletes_it() {
        let mut e = editor_with_area();
        e.set_tool(Tool::Text);
        click(&mut e, (300, 300), false);
        e.insert_text("ab");
        e.edit_key(EditKey::Finish);
        e.set_tool(Tool::Select);
        let b = e.shapes()[0].bbox();
        let inside = (b.x + b.w / 2, b.y + b.h / 2);
        click(&mut e, inside, false);
        click(&mut e, inside, true);
        assert!(e.is_editing());
        assert_eq!(
            e.static_shapes().count(),
            0,
            "edited shape is not drawn twice"
        );
        assert!(e.active_shape().is_some());
        e.edit_key(EditKey::Backspace);
        e.edit_key(EditKey::Backspace);
        e.edit_key(EditKey::Finish);
        assert!(e.shapes().is_empty());
        e.undo();
        assert_eq!(e.shapes()[0].text, "ab");
    }

    #[test]
    fn home_end_and_newline_move_within_lines() {
        let mut e = editor_with_area();
        e.set_tool(Tool::Text);
        click(&mut e, (300, 300), false);
        e.insert_text("ab");
        e.edit_key(EditKey::Newline);
        e.insert_text("cd");
        e.edit_key(EditKey::Home);
        e.insert_text(">");
        e.edit_key(EditKey::End);
        e.insert_text("<");
        e.edit_key(EditKey::Finish);
        assert_eq!(e.shapes()[0].text, "ab\n>cd<");
    }

    #[test]
    fn text_bg_toggle_applies_to_selected_text() {
        let mut e = editor_with_area();
        e.set_tool(Tool::Text);
        click(&mut e, (300, 300), false);
        e.insert_text("x");
        e.edit_key(EditKey::Finish);
        assert!(!e.shapes()[0].style.bg);
        e.toggle_text_bg();
        assert!(e.shapes()[0].style.bg);
        e.undo();
        assert!(!e.shapes()[0].style.bg);
    }
}
