//! The recording toolbar's settings: their icons, tooltips and states. Same
//! look as the drawing tools: icon buttons, with flyouts for choices.

use tiny_skia::{BlendMode, PathBuilder, Pixmap, Transform};

use super::{Group, Icon, Item};
use crate::config::{HighlightSize, RecordChoice, RecordOptions, Switch};
use crate::paint::{contrast, paint, round_stroke};
use crate::style::{COLOR_NAMES, PALETTE};
use crate::theme::{INK, PANEL};

/// Text on the frame-rate and quality buttons.
const VALUE_PT: f32 = 11.0;

pub(super) fn tooltip(choice: RecordChoice) -> String {
    match choice {
        RecordChoice::Fps(f) => format!("{f} fps"),
        RecordChoice::Quality(q) => q.name().into(),
        RecordChoice::Toggle(Switch::Mic) => "Record microphone".into(),
        RecordChoice::Toggle(Switch::Countdown) => "Count down 3-2-1".into(),
        RecordChoice::Toggle(Switch::Outline) => "Frame around the area".into(),
        RecordChoice::Toggle(Switch::Pointer) => "Highlight the mouse pointer".into(),
        RecordChoice::Toggle(Switch::Clicks) => "Show mouse clicks".into(),
        RecordChoice::HighlightColor(c)
        | RecordChoice::LeftColor(c)
        | RecordChoice::RightColor(c) => COLOR_NAMES[c].into(),
        RecordChoice::HighlightSize(s) => s.name().into(),
    }
}

pub(super) fn is_color(choice: RecordChoice) -> bool {
    matches!(
        choice,
        RecordChoice::HighlightColor(_) | RecordChoice::LeftColor(_) | RecordChoice::RightColor(_)
    )
}

/// Whether `item` would have no effect with the settings as they are.
pub(super) fn faded(item: Item, options: &RecordOptions) -> bool {
    match item {
        Item::Set(choice) => !options.applies(choice),
        Item::Group(Group::PointerColor | Group::PointerSize) => !options.highlight_cursor,
        Item::Group(Group::LeftColor | Group::RightColor) => !options.show_clicks,
        _ => false,
    }
}

/// Washes a button toward the panel colour without touching how see-through
/// the bar is, so no box shows around what is faded.
pub(super) fn fade(pm: &mut Pixmap, (x, y, w, h): (f32, f32, f32, f32)) {
    let mut veil = paint(PANEL, 165);
    veil.blend_mode = BlendMode::SourceAtop;
    if let Some(r) = tiny_skia::Rect::from_xywh(x, y, w, h) {
        pm.fill_rect(r, &veil, Transform::identity(), None);
    }
}

/// Radius of a highlight size's dot, as a share of the button.
fn dot_radius(size: HighlightSize) -> f32 {
    match size {
        HighlightSize::Small => 0.12,
        HighlightSize::Medium => 0.19,
        HighlightSize::Large => 0.27,
    }
}

impl Icon<'_> {
    /// A settings group's button: it shows the current value.
    pub(super) fn record_group(&mut self, group: Group, o: &RecordOptions) {
        match group {
            Group::Fps => self.value(&o.quality.fps(o.fps).to_string(), 13.0),
            Group::Quality => {
                let short = match o.quality {
                    crate::config::Quality::High => "High",
                    crate::config::Quality::Medium => "Med",
                    crate::config::Quality::Low => "Low",
                };
                self.value(short, VALUE_PT);
            }
            Group::PointerColor => self.swatch(PALETTE[o.highlight_color], 0.26, false),
            Group::PointerSize => self.size_ring(o.highlight_size),
            Group::LeftColor => self.lettered(o.left_click_color, "L"),
            Group::RightColor => self.lettered(o.right_click_color, "R"),
            Group::Shapes | Group::Draw | Group::Color | Group::Size => {}
        }
    }

    /// A switch, or one choice in a flyout.
    pub(super) fn record_choice(&mut self, choice: RecordChoice, o: &RecordOptions) {
        match choice {
            RecordChoice::Toggle(switch) => {
                match switch {
                    Switch::Mic => self.microphone(),
                    Switch::Countdown => self.countdown(),
                    Switch::Outline => self.frame_corners(),
                    Switch::Pointer => self.pointer(PALETTE[o.highlight_color]),
                    Switch::Clicks => self.ripple(),
                }
                if !o.is_on(switch) {
                    // Faded and struck through: off.
                    let (x, y, c) = self.cell;
                    fade(self.pm, (x, y, c, c));
                    self.stroke(&[(0.05, 0.95), (0.95, 0.05)], false);
                }
            }
            RecordChoice::HighlightColor(c)
            | RecordChoice::LeftColor(c)
            | RecordChoice::RightColor(c) => self.swatch(PALETTE[c], 0.24, o.has(choice)),
            RecordChoice::HighlightSize(s) => self.size_ring(s),
            // Drawn as text.
            RecordChoice::Fps(_) | RecordChoice::Quality(_) => {}
        }
    }

    /// `text` in the middle of the button.
    fn value(&mut self, text: &str, pt: f32) {
        let (x, y, c) = self.cell;
        let size = pt * self.s;
        let l = crate::text::layout(text, size);
        let ink = self.ink.clone();
        let at = (x + (c - l.w) / 2.0, y + (c - l.h) / 2.0);
        l.draw(
            &mut self.pm.as_mut(),
            at.0,
            at.1,
            size,
            &ink,
            Transform::identity(),
        );
    }

    /// A circle `radius` (share of the button) around unit-box point (`u`, `v`).
    fn circle(&self, u: f32, v: f32, radius: f32) -> Option<tiny_skia::Path> {
        let (x, y) = self.pt(u, v);
        PathBuilder::from_circle(x, y, self.cell.2 * radius)
    }

    fn size_ring(&mut self, size: HighlightSize) {
        let path = self.circle(0.5, 0.5, dot_radius(size));
        self.stroke_path(path);
    }

    /// A colour dot with a letter on it.
    fn lettered(&mut self, color: usize, letter: &str) {
        let rgb = PALETTE[color];
        self.swatch(rgb, 0.28, false);
        let ink = std::mem::replace(&mut self.ink, paint(contrast(rgb), 255));
        self.value(letter, VALUE_PT);
        self.ink = ink;
    }

    fn microphone(&mut self) {
        let (x, y) = self.pt(0.32, 0.0);
        let side = self.unit.2;
        if let Some(r) = tiny_skia::Rect::from_xywh(x, y, side * 0.36, side * 0.62) {
            let capsule = super::rounded_rect(r.x(), r.y(), r.width(), r.height(), side * 0.18);
            self.stroke_path(Some(capsule));
        }
        // The cradle, then the stand.
        let (a, c, b) = (self.pt(0.14, 0.42), self.pt(0.5, 1.18), self.pt(0.86, 0.42));
        let mut pb = PathBuilder::new();
        pb.move_to(a.0, a.1);
        pb.quad_to(c.0, c.1, b.0, b.1);
        self.stroke_path(pb.finish());
        self.stroke(&[(0.5, 0.8), (0.5, 1.0)], false);
    }

    /// A "3" in a ring.
    fn countdown(&mut self) {
        let ring = self.circle(0.5, 0.5, 0.27);
        self.stroke_path(ring);
        self.value("3", VALUE_PT);
    }

    fn frame_corners(&mut self) {
        for corner in [
            [(0.0, 0.35), (0.0, 0.08), (0.3, 0.08)],
            [(0.7, 0.08), (1.0, 0.08), (1.0, 0.35)],
            [(1.0, 0.65), (1.0, 0.92), (0.7, 0.92)],
            [(0.3, 0.92), (0.0, 0.92), (0.0, 0.65)],
        ] {
            self.stroke(&corner, false);
        }
    }

    /// The pointer with its halo.
    fn pointer(&mut self, rgb: u32) {
        if let Some(halo) = self.circle(0.34, 0.34, 0.25) {
            let ts = Transform::identity();
            let tint = paint(rgb, 170);
            self.pm
                .fill_path(&halo, &tint, tiny_skia::FillRule::Winding, ts, None);
        }
        self.fill(
            &[
                (0.34, 0.24),
                (0.34, 0.92),
                (0.51, 0.76),
                (0.65, 1.05),
                (0.77, 0.99),
                (0.63, 0.71),
                (0.87, 0.71),
            ],
            None,
        );
    }

    /// A click and the rings spreading from it.
    fn ripple(&mut self) {
        if let Some(dot) = self.circle(0.5, 0.5, 0.07) {
            let ink = self.ink.clone();
            self.fill_path(Some(dot), &ink);
        }
        let inner = self.circle(0.5, 0.5, 0.17);
        self.stroke_path(inner);
        if let Some(outer) = self.circle(0.5, 0.5, 0.29) {
            let faint = paint(INK, 110);
            let line = round_stroke(1.4 * self.s);
            let ts = Transform::identity();
            self.pm.stroke_path(&outer, &faint, &line, ts, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{State, Toolbar, flyout, layout};
    use super::*;
    use crate::config::{Config, Preset, Quality};
    use crate::geom::{Rect, Scale};
    use crate::style::{Tool, Width};

    const SCREEN: Rect = Rect {
        x: 0,
        y: 0,
        w: 3000,
        h: 1900,
    };
    const GROUPS: [Group; 6] = [
        Group::Fps,
        Group::Quality,
        Group::PointerColor,
        Group::PointerSize,
        Group::LeftColor,
        Group::RightColor,
    ];

    fn bar() -> Toolbar {
        layout(Rect::new(600, 300, 1200, 500), SCREEN, Scale(2.0), true)
    }

    fn state(options: RecordOptions) -> State {
        State {
            tool: Tool::Select,
            color: 0,
            width: Width::M,
            text_bg: false,
            preset: Preset::Custom,
            shape_tool: Tool::Rect,
            draw_tool: Tool::Pen,
            open: None,
            can_undo: false,
            can_redo: false,
            hover: None,
            record: Some(options),
        }
    }

    #[test]
    fn every_recording_setting_is_on_the_toolbar_or_in_a_flyout() {
        let bar = bar();
        let mut items: Vec<Item> = bar.items.iter().map(|&(i, _)| i).collect();
        assert_eq!(items[items.len() - 2..], [Item::Record, Item::Close]);
        for g in GROUPS {
            assert!(items.contains(&Item::Group(g)), "{g:?}");
            items.extend(g.items());
        }
        // 5 switches, 2 rates, 3 qualities, 3 sizes, 3 rows of colours.
        let settings = items.iter().filter(|i| matches!(i, Item::Set(_)));
        assert_eq!(settings.count(), 5 + 2 + 3 + 3 + 3 * PALETTE.len());
        for item in items {
            assert!(super::super::tooltip(item).len() > 2, "{item:?}");
        }
    }

    #[test]
    fn flyouts_fit_the_screen_and_named_choices_fit_their_buttons() {
        let bar = bar();
        for g in GROUPS {
            let f = flyout(&bar, g, SCREEN, Scale(2.0)).unwrap();
            assert_eq!(f.rect.intersect(&SCREEN), f.rect, "{g:?}");
            assert_eq!(f.rect.y, bar.rect.bottom() + 8, "{g:?}");
            for &(item, r) in &f.items {
                if let Some(t) = super::super::label_of(item) {
                    let w = crate::text::layout(&t, super::super::TEXT_PT * 2.0).w;
                    assert!(w < r.w as f32, "{t}: {w} in {}", r.w);
                }
            }
        }
    }

    #[test]
    fn what_has_no_effect_is_faded() {
        let mut o = Config::default().record_options();
        let color = Item::Group(Group::LeftColor);
        assert!(!faded(color, &o));
        o.show_clicks = false;
        assert!(faded(color, &o));
        assert!(!faded(Item::Group(Group::PointerColor), &o));
        // The switch itself stays usable.
        assert!(!faded(Item::Set(RecordChoice::Toggle(Switch::Clicks)), &o));
        o.quality = Quality::Low;
        assert!(faded(Item::Set(RecordChoice::Fps(60)), &o));
    }

    #[test]
    fn a_switch_looks_different_when_off() {
        let bar = bar();
        let mut o = Config::default().record_options();
        let picture = |o: RecordOptions| bar.draw(&state(o), Scale(2.0)).data().to_vec();
        for s in [
            Switch::Mic,
            Switch::Countdown,
            Switch::Outline,
            Switch::Pointer,
            Switch::Clicks,
        ] {
            let before = picture(o);
            o.apply(RecordChoice::Toggle(s));
            assert_ne!(picture(o), before, "{s:?}");
        }
    }

    /// `SHOTLY_BAR_OUT=/path.png cargo test dump_record_bar -- --ignored`
    /// writes the toolbar and its flyouts to look at.
    #[test]
    #[ignore = "writes a preview file"]
    fn dump_record_bar() {
        let Some(path) = std::env::var_os("SHOTLY_BAR_OUT") else {
            return;
        };
        let bar = bar();
        let mut o = Config::default().record_options();
        let mut shots = vec![bar.draw(&state(o), Scale(2.0))];
        // The other state of every switch, with Low quality.
        for s in [
            Switch::Mic,
            Switch::Countdown,
            Switch::Outline,
            Switch::Clicks,
        ] {
            o.apply(RecordChoice::Toggle(s));
        }
        o.quality = Quality::Low;
        shots.push(bar.draw(&state(o), Scale(2.0)));
        for g in GROUPS {
            let f = flyout(&bar, g, SCREEN, Scale(2.0)).unwrap();
            shots.push(f.draw(&state(o), Scale(2.0)));
        }
        let w = shots.iter().map(Pixmap::width).max().unwrap() + 40;
        let h = shots.iter().map(|p| p.height() + 16).sum::<u32>() + 24;
        let mut sheet = Pixmap::new(w, h).unwrap();
        sheet.fill(tiny_skia::Color::from_rgba8(110, 120, 135, 255));
        let mut y = 20;
        for p in shots {
            sheet.draw_pixmap(
                20,
                y,
                p.as_ref(),
                &tiny_skia::PixmapPaint::default(),
                Transform::identity(),
                None,
            );
            y += p.height() as i32 + 16;
        }
        sheet.save_png(path).unwrap();
    }
}
