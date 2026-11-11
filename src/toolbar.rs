//! Toolbar next to the selection: layout, hit-testing and drawing.
//! Icons are drawn as vector paths, so no image assets or fonts are needed.

use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};

use crate::config::Preset;
use crate::geom::{Point, Rect, Scale};
use crate::paint::{paint, round_stroke};
use crate::shape::Kind;
use crate::style::{COLOR_NAMES, PALETTE, Tool, Width};
use crate::theme::{self, INK, PANEL};

/// A toolbar button that opens a flyout with related choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    /// Rectangle, ellipse, line.
    Shapes,
    /// Pen, highlighter.
    Draw,
    Color,
    /// S/M/L plus the text-box toggle.
    Size,
}

impl Group {
    pub fn items(self) -> Vec<Item> {
        match self {
            Self::Shapes => [Tool::Rect, Tool::Ellipse, Tool::Line]
                .map(Item::Tool)
                .to_vec(),
            Self::Draw => [Tool::Pen, Tool::Highlighter].map(Item::Tool).to_vec(),
            Self::Color => (0..PALETTE.len()).map(Item::Color).collect(),
            Self::Size => Width::ALL
                .iter()
                .map(|&w| Item::Width(w))
                .chain([Item::TextBg])
                .collect(),
        }
    }

    /// The tool group a tool belongs to, if it is not a button of its own.
    pub fn of_tool(t: Tool) -> Option<Self> {
        match t {
            Tool::Rect | Tool::Ellipse | Tool::Line => Some(Self::Shapes),
            Tool::Pen | Tool::Highlighter => Some(Self::Draw),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Group(Group),
    Tool(Tool),
    Color(usize),
    Width(Width),
    /// Filled box behind text on/off.
    TextBg,
    Undo,
    Redo,
    Copy,
    Save,
    /// Start recording the selection (record mode only).
    Record,
    Close,
    /// Full screen / Window / Custom, in the strip at the top.
    Preset(Preset),
}

/// Text on a preset button; `None` for buttons drawn as icons.
fn label_of(item: Item) -> Option<String> {
    match item {
        Item::Preset(p) => Some(p.name().into()),
        _ => None,
    }
}

/// What the toolbar should highlight.
#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub tool: Tool,
    pub color: usize,
    pub width: Width,
    pub text_bg: bool,
    pub preset: Preset,
    /// Last tool picked in each tool group (shown on its button).
    pub shape_tool: Tool,
    pub draw_tool: Tool,
    /// Group whose flyout is open.
    pub open: Option<Group>,
    pub can_undo: bool,
    pub can_redo: bool,
    pub hover: Option<Item>,
}

pub struct Toolbar {
    /// Window pixels.
    pub rect: Rect,
    pub items: Vec<(Item, Rect)>,
}

const CELL_PT: f64 = 30.0;
const PAD_PT: f64 = 4.0;
const GAP_PT: f64 = 10.0;
const MARGIN_PT: f64 = 8.0;
/// Space between the toolbar and an open flyout.
const FLYOUT_GAP_PT: f64 = 4.0;

fn groups(record: bool) -> Vec<Vec<Item>> {
    if record {
        return vec![vec![Item::Record, Item::Close]];
    }
    vec![
        vec![
            Item::Tool(Tool::Select),
            Item::Group(Group::Shapes),
            Item::Tool(Tool::Arrow),
            Item::Group(Group::Draw),
            Item::Tool(Tool::Text),
            Item::Tool(Tool::Blur),
        ],
        vec![Item::Group(Group::Color), Item::Group(Group::Size)],
        vec![Item::Undo, Item::Redo],
        vec![Item::Copy, Item::Save, Item::Close],
    ]
}

/// Tooltip text for a button, with its keyboard shortcut.
pub fn tooltip(item: Item) -> String {
    match item {
        Item::Group(Group::Shapes) => "Shapes (R, O, L)".into(),
        Item::Group(Group::Draw) => "Draw (P, H)".into(),
        Item::Group(Group::Color) => "Color".into(),
        Item::Group(Group::Size) => "Size and text background".into(),
        Item::Tool(t) => format!("{} ({})", t.name(), t.key()),
        Item::Color(i) => COLOR_NAMES[i].into(),
        Item::Width(Width::S) => "Small".into(),
        Item::Width(Width::M) => "Medium".into(),
        Item::Width(Width::L) => "Large".into(),
        Item::TextBg => "Text background".into(),
        Item::Undo => "Undo (⌘Z)".into(),
        Item::Redo => "Redo (⇧⌘Z)".into(),
        Item::Copy => "Copy to clipboard (↩ or ⌘C)".into(),
        Item::Save => "Save to file (⌘S)".into(),
        Item::Record => "Start recording (↩)".into(),
        Item::Preset(Preset::Full) => {
            format!("Full screen: the whole display ({})", Preset::Full.key())
        }
        Item::Preset(Preset::Window) => format!("Window: click one ({})", Preset::Window.key()),
        Item::Preset(Preset::Custom) => {
            format!(
                "Custom: drag an area with the mouse ({})",
                Preset::Custom.key()
            )
        }
        Item::Close => "Cancel (Esc)".into(),
    }
}

/// Where a tooltip of `size` goes for a hovered button: under its bar, or
/// above it when there is no room, never over the other bar (an open flyout).
pub fn place_tooltip(
    size: (i32, i32),
    item: Rect,
    bar: Rect,
    other: Option<Rect>,
    bounds: Rect,
    gap: i32,
) -> Rect {
    let (w, h) = size;
    let x = item.centered_x(w, &bounds, gap);
    let below = Rect::new(x, bar.bottom() + gap, w, h);
    let above = Rect::new(x, bar.y - gap - h, w, h);
    let fits =
        |r: &Rect| r.intersect(&bounds) == *r && other.is_none_or(|o| r.intersect(&o).is_empty());
    if fits(&below) {
        below
    } else if fits(&above) {
        above
    } else {
        // Cramped: tuck it just under the button's own bar, kept on screen.
        Rect::new(x, below.y.min(bounds.bottom() - h), w, h)
    }
}

/// Lays out `groups` in one row starting at (`x`, `y`).
fn row(groups: Vec<Vec<Item>>, x: i32, y: i32, cell: i32, pad: i32, gap: i32) -> Toolbar {
    let n: i32 = groups.iter().map(|g| g.len() as i32).sum();
    let w = 2 * pad + n * cell + (groups.len() as i32 - 1) * gap;
    let mut items = Vec::with_capacity(n as usize);
    let mut cx = x + pad;
    for g in groups {
        for item in g {
            items.push((item, Rect::new(cx, y + pad, cell, cell)));
            cx += cell;
        }
        cx += gap;
    }
    Toolbar {
        rect: Rect::new(x, y, w, cell + 2 * pad),
        items,
    }
}

/// The flyout for `group`, under its button (or above the toolbar when
/// there is no room below).
pub fn flyout(tb: &Toolbar, group: Group, bounds: Rect, scale: Scale) -> Option<Toolbar> {
    let (cell, pad, margin, gap) = (
        scale.px(CELL_PT),
        scale.px(PAD_PT),
        scale.px(MARGIN_PT),
        scale.px(FLYOUT_GAP_PT),
    );
    let button = tb.items.iter().find(|(i, _)| *i == Item::Group(group))?.1;
    let items = group.items();
    let w = 2 * pad + items.len() as i32 * cell;
    let h = cell + 2 * pad;
    let x = button.centered_x(w, &bounds, margin);
    let below = tb.rect.bottom() + gap;
    let y = if below + h <= bounds.bottom() - margin {
        below
    } else {
        tb.rect.y - gap - h
    };
    Some(row(vec![items], x, y, cell, pad, 0))
}

const TEXT_PT: f32 = 12.5;
const CELL_PAD_X_PT: f64 = 10.0;
const CELL_GAP_PT: f64 = 2.0;
/// Clear of the notch on `MacBook` displays, which is about 32 pt tall.
const STRIP_TOP_PT: f64 = 48.0;

/// A row of text buttons whose top is at `y`, centred on `centre_x` and kept
/// inside `bounds`.
fn text_bar(items: &[Item], y: i32, centre_x: i32, bounds: Rect, scale: Scale) -> Toolbar {
    let (cell, pad, gap) = (scale.px(CELL_PT), scale.px(PAD_PT), scale.px(CELL_GAP_PT));
    let (cell_pad, margin) = (scale.px(CELL_PAD_X_PT), scale.px(MARGIN_PT));
    let size = scale.pxf(TEXT_PT);
    let widths: Vec<i32> = items
        .iter()
        .map(|&i| {
            let text = label_of(i).unwrap_or_default();
            crate::text::layout(&text, size).w.ceil() as i32 + 2 * cell_pad
        })
        .collect();
    let total = widths.iter().sum::<i32>() + gap * (items.len() as i32 - 1).max(0) + 2 * pad;
    let x = (centre_x - total / 2).clamp(
        bounds.x + margin,
        (bounds.right() - total - margin).max(bounds.x),
    );
    let mut cx = x + pad;
    let cells = items
        .iter()
        .zip(&widths)
        .map(|(&item, &w)| {
            let r = Rect::new(cx, y + pad, w, cell);
            cx += w + gap;
            (item, r)
        })
        .collect();
    Toolbar {
        rect: Rect::new(x, y, total, cell + 2 * pad),
        items: cells,
    }
}

/// The Full screen / Window / Custom strip at the top of a display.
pub fn strip(bounds: Rect, scale: Scale) -> Toolbar {
    let items = Preset::ALL.map(Item::Preset);
    let y = bounds.y + scale.px(STRIP_TOP_PT);
    text_bar(&items, y, bounds.x + bounds.w / 2, bounds, scale)
}

/// Places the toolbar under `area`, or above it, or inside its bottom edge,
/// whichever fits inside `bounds` first.
pub fn layout(area: Rect, bounds: Rect, scale: Scale, record: bool) -> Toolbar {
    let (cell, pad, gap, margin) = (
        scale.px(CELL_PT),
        scale.px(PAD_PT),
        scale.px(GAP_PT),
        scale.px(MARGIN_PT),
    );
    let probe = row(groups(record), 0, 0, cell, pad, gap);
    let (w, h) = (probe.rect.w, probe.rect.h);

    let x = area.centered_x(w, &bounds, margin);
    let below = area.bottom() + margin;
    let above = area.y - margin - h;
    let y = if below + h <= bounds.bottom() - margin {
        below
    } else if above >= bounds.y + margin {
        above
    } else {
        (area.bottom() - margin - h).max(bounds.y + margin)
    };
    row(groups(record), x, y, cell, pad, gap)
}

impl Toolbar {
    pub fn hit(&self, p: Point) -> Option<Item> {
        self.items
            .iter()
            .find(|(_, r)| r.contains(p))
            .map(|&(i, _)| i)
    }

    pub fn draw(&self, st: &State, scale: Scale) -> Pixmap {
        let s = scale.pxf(1.0);
        let mut pm = Pixmap::new(self.rect.w.max(1) as u32, self.rect.h.max(1) as u32).unwrap();
        let (ox, oy) = (self.rect.x as f32, self.rect.y as f32);
        let bg = rounded_rect(0.0, 0.0, self.rect.w as f32, self.rect.h as f32, 9.0 * s);
        pm.fill_path(
            &bg,
            &paint(PANEL, theme::PANEL_ALPHA),
            FillRule::Winding,
            Transform::identity(),
            None,
        );

        for &(item, r) in &self.items {
            let (x, y, c) = (r.x as f32 - ox, r.y as f32 - oy, r.w as f32);
            let ch = r.h as f32;
            let active = match item {
                Item::Preset(p) => p == st.preset,
                Item::Group(g) => st.open == Some(g) || Group::of_tool(st.tool) == Some(g),
                Item::Tool(t) => t == st.tool,
                Item::Width(w) => w == st.width,
                Item::TextBg => st.text_bg,
                _ => false,
            };
            if active || st.hover == Some(item) {
                let alpha = if active { 50 } else { 24 };
                let hl = rounded_rect(x + 1.0, y + 1.0, c - 2.0, ch - 2.0, 6.0 * s);
                pm.fill_path(
                    &hl,
                    &paint(theme::HIGHLIGHT, alpha),
                    FillRule::Winding,
                    Transform::identity(),
                    None,
                );
            }
            if let Some(label) = label_of(item) {
                let size = TEXT_PT * s;
                let l = crate::text::layout(&label, size);
                l.draw(
                    &mut pm.as_mut(),
                    x + (c - l.w) / 2.0,
                    y + (ch - l.h) / 2.0,
                    size,
                    &paint(INK, 235),
                    Transform::identity(),
                );
                continue;
            }
            let enabled = match item {
                Item::Undo => st.can_undo,
                Item::Redo => st.can_redo,
                _ => true,
            };
            draw_icon(&mut pm, item, (x, y, c), st, s, enabled);
        }
        pm
    }
}

fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> tiny_skia::Path {
    crate::paint::rounded_rect(tiny_skia::Rect::from_xywh(x, y, w, h).unwrap(), r)
}

/// Draws `item`'s icon inside the cell (`x`, `y`, size `c`) of `pm`.
fn draw_icon(
    pm: &mut Pixmap,
    item: Item,
    cell: (f32, f32, f32),
    st: &State,
    s: f32,
    enabled: bool,
) {
    let mut ic = Icon::new(pm, cell, s, enabled);
    match item {
        Item::Group(g) => {
            match g {
                Group::Shapes => draw_icon(ic.pm, Item::Tool(st.shape_tool), cell, st, s, enabled),
                Group::Draw => draw_icon(ic.pm, Item::Tool(st.draw_tool), cell, st, s, enabled),
                Group::Size => draw_icon(ic.pm, Item::Width(st.width), cell, st, s, enabled),
                Group::Color => ic.swatch(PALETTE[st.color], 0.26, false),
            }
            ic.chevron();
        }
        Item::Tool(Tool::Select) => ic.select(),
        Item::Tool(Tool::Rect) => {
            ic.stroke(&[(0.0, 0.12), (1.0, 0.12), (1.0, 0.88), (0.0, 0.88)], true);
        }
        Item::Tool(Tool::Ellipse) => ic.ellipse(),
        Item::Tool(Tool::Line) => ic.stroke(&[(0.0, 1.0), (1.0, 0.0)], false),
        Item::Tool(Tool::Arrow) => ic.arrow(),
        Item::Tool(Tool::Pen) => ic.pen(),
        Item::Tool(Tool::Highlighter) => ic.highlighter(),
        Item::Tool(Tool::Text) => ic.letter_t(|u, v| (u, v), None),
        Item::Tool(Tool::Blur) => ic.mosaic(enabled),
        Item::Color(i) => ic.swatch(PALETTE[i], 0.24, i == st.color),
        Item::Width(w) => ic.width_line(w),
        Item::TextBg => ic.text_bg(),
        Item::Undo => ic.curved_arrow(false),
        Item::Redo => ic.curved_arrow(true),
        Item::Copy => ic.copy(),
        Item::Save => ic.save(),
        Item::Record => ic.record_dot(),
        // Drawn as text by `Toolbar::draw`.
        Item::Preset(_) => {}
        Item::Close => {
            ic.stroke(&[(0.1, 0.1), (0.9, 0.9)], false);
            ic.stroke(&[(0.9, 0.1), (0.1, 0.9)], false);
        }
    }
}

/// Paints one toolbar icon. Shapes are given in a unit box (0..1 on both
/// axes) inset inside the button cell, so every icon has the same margins.
struct Icon<'a> {
    pm: &'a mut Pixmap,
    /// Button cell: x, y, size (pixels).
    cell: (f32, f32, f32),
    /// Unit box: x, y, size (pixels).
    unit: (f32, f32, f32),
    /// Backing scale, for line widths.
    s: f32,
    ink: Paint<'static>,
    line: tiny_skia::Stroke,
}

impl<'a> Icon<'a> {
    fn new(pm: &'a mut Pixmap, cell: (f32, f32, f32), s: f32, enabled: bool) -> Self {
        let (x, y, c) = cell;
        let inset = c * 0.27;
        Self {
            pm,
            cell,
            unit: (x + inset, y + inset, c - 2.0 * inset),
            s,
            ink: paint(INK, if enabled { 235 } else { 80 }),
            line: round_stroke(1.6 * s),
        }
    }

    /// Unit-box coordinates → pixels.
    fn pt(&self, u: f32, v: f32) -> (f32, f32) {
        let (bx, by, bs) = self.unit;
        (bx + u * bs, by + v * bs)
    }

    fn poly(&self, pts: &[(f32, f32)], close: bool) -> Option<tiny_skia::Path> {
        let (first, rest) = pts.split_first()?;
        let mut pb = PathBuilder::new();
        let (x, y) = self.pt(first.0, first.1);
        pb.move_to(x, y);
        for &(u, v) in rest {
            let (x, y) = self.pt(u, v);
            pb.line_to(x, y);
        }
        if close {
            pb.close();
        }
        pb.finish()
    }

    fn stroke_path(&mut self, path: Option<tiny_skia::Path>) {
        if let Some(p) = path {
            self.pm
                .stroke_path(&p, &self.ink, &self.line, Transform::identity(), None);
        }
    }

    fn fill_path(&mut self, path: Option<tiny_skia::Path>, paint: &Paint) {
        if let Some(p) = path {
            self.pm
                .fill_path(&p, paint, FillRule::Winding, Transform::identity(), None);
        }
    }

    /// Outline through unit-box points.
    fn stroke(&mut self, pts: &[(f32, f32)], close: bool) {
        let path = self.poly(pts, close);
        self.stroke_path(path);
    }

    /// Solid polygon in `paint` (ink if `None`).
    fn fill(&mut self, pts: &[(f32, f32)], paint: Option<&Paint>) {
        let path = self.poly(pts, true);
        let paint = paint.cloned().unwrap_or_else(|| self.ink.clone());
        self.fill_path(path, &paint);
    }

    fn select(&mut self) {
        self.fill(
            &[
                (0.15, 0.0),
                (0.15, 0.85),
                (0.38, 0.64),
                (0.55, 1.0),
                (0.7, 0.93),
                (0.53, 0.58),
                (0.83, 0.58),
            ],
            None,
        );
    }

    fn ellipse(&mut self) {
        let (x, y) = self.pt(0.0, 0.08);
        let bs = self.unit.2;
        let oval = tiny_skia::Rect::from_xywh(x, y, bs, bs * 0.84).and_then(PathBuilder::from_oval);
        self.stroke_path(oval);
    }

    fn arrow(&mut self) {
        self.stroke(&[(0.0, 1.0), (0.85, 0.15)], false);
        self.fill(&[(1.0, 0.0), (0.45, 0.13), (0.87, 0.55)], None);
    }

    fn pen(&mut self) {
        let (a, c1, c2, e) = (
            self.pt(0.0, 0.8),
            self.pt(0.3, -0.2),
            self.pt(0.55, 1.3),
            self.pt(1.0, 0.2),
        );
        let mut pb = PathBuilder::new();
        pb.move_to(a.0, a.1);
        pb.cubic_to(c1.0, c1.1, c2.0, c2.1, e.0, e.1);
        self.stroke_path(pb.finish());
    }

    /// Chisel-tip marker, tilted.
    fn highlighter(&mut self) {
        self.stroke(&[(0.55, 0.0), (1.0, 0.45), (0.45, 1.0), (0.0, 0.55)], true);
        self.fill(&[(0.0, 0.55), (0.45, 1.0), (0.12, 1.0), (0.0, 0.88)], None);
    }

    /// 3×3 mosaic with alternating cells filled.
    fn mosaic(&mut self, enabled: bool) {
        let (bx, by, bs) = self.unit;
        let cell = bs / 3.0;
        for gy in 0..3u8 {
            for gx in 0..3u8 {
                let alpha = match ((gx + gy) % 2 == 0, enabled) {
                    (_, false) => 60,
                    (true, true) => 235,
                    (false, true) => 90,
                };
                let gap = 0.5 * self.s;
                let r = tiny_skia::Rect::from_xywh(
                    bx + f32::from(gx) * cell,
                    by + f32::from(gy) * cell,
                    cell - gap,
                    cell - gap,
                );
                if let Some(r) = r {
                    self.pm
                        .fill_rect(r, &paint(INK, alpha), Transform::identity(), None);
                }
            }
        }
    }

    /// Colour dot (with an outline so black/white stay visible), ringed
    /// when it is the current colour.
    fn swatch(&mut self, rgb: u32, radius: f32, ring: bool) {
        let (x, y, c) = self.cell;
        let (cx, cy) = (x + c / 2.0, y + c / 2.0);
        if let Some(dot) = PathBuilder::from_circle(cx, cy, c * radius) {
            self.pm.fill_path(
                &dot,
                &paint(rgb, 255),
                FillRule::Winding,
                Transform::identity(),
                None,
            );
            let outline = round_stroke(self.s);
            self.pm.stroke_path(
                &dot,
                &paint(theme::SWATCH_OUTLINE, 160),
                &outline,
                Transform::identity(),
                None,
            );
        }
        if ring {
            self.stroke_path(PathBuilder::from_circle(cx, cy, c * 0.36));
        }
    }

    fn width_line(&mut self, w: Width) {
        let path = self.poly(&[(0.0, 0.5), (1.0, 0.5)], false);
        let line = round_stroke((w.pt(Kind::Pen) * self.s).min(self.unit.2 * 0.5));
        if let Some(p) = path {
            self.pm
                .stroke_path(&p, &self.ink, &line, Transform::identity(), None);
        }
    }

    /// A "T" from two strokes; `map` places it within the unit box and
    /// `paint` overrides the ink.
    fn letter_t(&mut self, map: impl Fn(f32, f32) -> (f32, f32), paint: Option<Paint<'static>>) {
        let p = |u, v| {
            let (u, v) = map(u, v);
            self.pt(u, v)
        };
        let (a, b, c, d) = (p(0.1, 0.05), p(0.9, 0.05), p(0.5, 0.05), p(0.5, 0.95));
        let mut pb = PathBuilder::new();
        pb.move_to(a.0, a.1);
        pb.line_to(b.0, b.1);
        pb.move_to(c.0, c.1);
        pb.line_to(d.0, d.1);
        if let Some(path) = pb.finish() {
            let paint = paint.unwrap_or_else(|| self.ink.clone());
            self.pm
                .stroke_path(&path, &paint, &self.line, Transform::identity(), None);
        }
    }

    /// A light rounded box with a dark "T" in it.
    fn text_bg(&mut self) {
        let (bx, by, bs) = self.unit;
        let bg = rounded_rect(bx - 0.1 * bs, by, bs * 1.2, bs, 3.0 * self.s);
        let ink = self.ink.clone();
        self.fill_path(Some(bg), &ink);
        self.letter_t(
            |u, v| (0.2 + u * 0.6, 0.18 + v * 0.64),
            Some(paint(PANEL, 255)),
        );
    }

    /// Undo arrow, mirrored for redo.
    fn curved_arrow(&mut self, mirror: bool) {
        let m = |u: f32| if mirror { 1.0 - u } else { u };
        let (s0, c0, e0, l0) = (
            self.pt(m(0.95), 0.95),
            self.pt(m(0.95), 0.32),
            self.pt(m(0.5), 0.32),
            self.pt(m(0.08), 0.32),
        );
        let mut pb = PathBuilder::new();
        pb.move_to(s0.0, s0.1);
        pb.quad_to(c0.0, c0.1, e0.0, e0.1);
        pb.line_to(l0.0, l0.1);
        self.stroke_path(pb.finish());
        self.stroke(&[(m(0.33), 0.08), (m(0.08), 0.32), (m(0.33), 0.56)], false);
    }

    fn copy(&mut self) {
        let front = [(0.0, 0.3), (0.7, 0.3), (0.7, 1.0), (0.0, 1.0)];
        self.stroke(&[(0.3, 0.0), (1.0, 0.0), (1.0, 0.7), (0.3, 0.7)], true);
        self.fill(&front, Some(&paint(PANEL, 255)));
        self.stroke(&front, true);
    }

    fn save(&mut self) {
        self.stroke(&[(0.5, 0.0), (0.5, 0.62)], false);
        self.stroke(&[(0.25, 0.38), (0.5, 0.63), (0.75, 0.38)], false);
        self.stroke(&[(0.0, 0.7), (0.0, 1.0), (1.0, 1.0), (1.0, 0.7)], false);
    }

    /// Red dot: start recording.
    fn record_dot(&mut self) {
        let (x, y, c) = self.cell;
        if let Some(dot) = PathBuilder::from_circle(x + c / 2.0, y + c / 2.0, c * 0.28) {
            self.pm.fill_path(
                &dot,
                &paint(0xFF3B30, 255),
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }

    /// ▾ in the corner: this button opens more choices.
    fn chevron(&mut self) {
        let (x, y, c) = self.cell;
        let mut pb = PathBuilder::new();
        pb.move_to(x + c * 0.76, y + c * 0.80);
        pb.line_to(x + c * 0.92, y + c * 0.80);
        pb.line_to(x + c * 0.84, y + c * 0.90);
        pb.close();
        self.fill_path(pb.finish(), &paint(INK, 200));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect {
        x: 0,
        y: 0,
        w: 3000,
        h: 2000,
    };

    #[test]
    fn sits_below_selection_when_room() {
        let tb = layout(Rect::new(1000, 500, 800, 400), SCREEN, Scale(2.0), false);
        assert_eq!(tb.rect.y, 900 + 16);
        assert!(tb.rect.x >= 16 && tb.rect.right() <= 3000 - 16);
    }

    #[test]
    fn flips_above_then_inside() {
        let tb = layout(Rect::new(100, 600, 800, 1400), SCREEN, Scale(2.0), false);
        assert_eq!(tb.rect.bottom(), 600 - 16);
        let full = layout(SCREEN, SCREEN, Scale(2.0), false);
        assert_eq!(full.rect.bottom(), 2000 - 16);
    }

    #[test]
    fn hit_finds_items() {
        let tb = layout(Rect::new(1000, 500, 800, 400), SCREEN, Scale(2.0), false);
        let (item, r) = tb.items[1];
        assert_eq!(item, Item::Group(Group::Shapes));
        assert_eq!(tb.hit(Point::new(r.x + 5, r.y + 5)), Some(item));
        assert_eq!(tb.hit(Point::new(tb.rect.x + 1, tb.rect.y + 1)), None);
        assert_eq!(tb.items.last().unwrap().0, Item::Close);
        assert_eq!(tb.items.len(), 13);
    }

    #[test]
    fn record_mode_toolbar_has_only_record_and_close() {
        let tb = layout(Rect::new(1000, 500, 800, 400), SCREEN, Scale(2.0), true);
        let items: Vec<Item> = tb.items.iter().map(|&(i, _)| i).collect();
        assert_eq!(items, vec![Item::Record, Item::Close]);
        assert!(tooltip(Item::Record).contains("recording"));
    }

    #[test]
    fn strip_is_centred_at_the_top_below_the_notch() {
        let bounds = SCREEN;
        let strip = strip(bounds, Scale(2.0));
        let items: Vec<Item> = strip.items.iter().map(|&(i, _)| i).collect();
        assert_eq!(items, Preset::ALL.map(Item::Preset).to_vec());
        assert!(strip.rect.y >= 96, "below the notch: {:?}", strip.rect);
        let mid = strip.rect.x + strip.rect.w / 2;
        assert!((mid - bounds.w / 2).abs() <= 2, "{mid}");
        // Buttons are wide enough for their text and do not overlap.
        for w in strip.items.windows(2) {
            assert!(w[0].1.right() <= w[1].1.x);
        }
        let (_, first) = strip.items[0];
        assert_eq!(
            strip.hit(Point::new(first.x + 3, first.y + 3)),
            Some(Item::Preset(Preset::Full))
        );
    }

    #[test]
    fn every_button_has_a_tooltip() {
        let tb = layout(Rect::new(1000, 500, 800, 400), SCREEN, Scale(2.0), false);
        let mut items: Vec<Item> = tb.items.iter().map(|&(i, _)| i).collect();
        for g in [Group::Shapes, Group::Draw, Group::Color, Group::Size] {
            items.extend(g.items());
        }
        items.extend(Preset::ALL.map(Item::Preset));
        for item in items {
            assert!(tooltip(item).len() > 2, "{item:?}");
        }
        assert_eq!(tooltip(Item::Tool(Tool::Rect)), "Rectangle (R)");
        assert_eq!(tooltip(Item::Color(4)), "Blue");
    }

    #[test]
    fn tooltip_goes_below_then_above_and_avoids_the_flyout() {
        let bounds = SCREEN;
        let bar = Rect::new(600, 900, 800, 76);
        let item = Rect::new(700, 904, 60, 60);
        let t = place_tooltip((200, 40), item, bar, None, bounds, 12);
        assert_eq!(t.y, bar.bottom() + 12);
        assert_eq!(t.x, 700 + 30 - 100);
        // Flyout open right under the bar: tooltip moves above.
        let flyout = Rect::new(650, 984, 400, 76);
        let t = place_tooltip((200, 40), item, bar, Some(flyout), bounds, 12);
        assert_eq!(t.bottom(), bar.y - 12);
        // Bar at the bottom edge: above too. Near the left edge: clamped.
        let low = Rect::new(0, 1900, 800, 76);
        let t = place_tooltip(
            (200, 40),
            Rect::new(10, 1904, 60, 60),
            low,
            None,
            bounds,
            12,
        );
        assert_eq!((t.x, t.bottom()), (12, 1900 - 12));
    }

    #[test]
    fn flyout_opens_under_its_button_or_above_toolbar() {
        let tb = layout(Rect::new(1000, 500, 800, 400), SCREEN, Scale(2.0), false);
        let button = tb
            .items
            .iter()
            .find(|(i, _)| *i == Item::Group(Group::Color))
            .unwrap()
            .1;
        let f = flyout(&tb, Group::Color, SCREEN, Scale(2.0)).unwrap();
        assert_eq!(f.items.len(), 8);
        assert_eq!(f.rect.y, tb.rect.bottom() + 8);
        assert!(f.rect.x <= button.x && f.rect.right() >= button.right());
        assert_eq!(
            f.hit(Point::new(f.items[3].1.x + 2, f.items[3].1.y + 2)),
            Some(Item::Color(3))
        );

        // Toolbar pinned to the bottom edge: flyout goes above it.
        let low = layout(Rect::new(100, 600, 800, 1400), SCREEN, Scale(2.0), false);
        let low = Toolbar {
            rect: Rect::new(low.rect.x, 1900, low.rect.w, low.rect.h),
            items: low.items,
        };
        let f = flyout(&low, Group::Size, SCREEN, Scale(2.0)).unwrap();
        assert_eq!(f.rect.bottom(), 1900 - 8);
        assert_eq!(f.items.last().unwrap().0, Item::TextBg);
    }
}
