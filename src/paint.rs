//! Draws shapes with tiny-skia. Used for both the on-screen overlay and the
//! exported image, so what you see is what you get.

use tiny_skia::{
    BlendMode, FillRule, LineCap, LineJoin, Paint, PathBuilder, PixmapMut, Stroke, Transform,
};

use crate::geom::{Point, Rect};
use crate::shape::{Geom, Kind, Shape, arrow_geometry};

pub fn paint(rgb: u32, alpha: u8) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, alpha);
    p.anti_alias = true;
    p
}

pub fn round_stroke(width: f32) -> Stroke {
    Stroke {
        width,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    }
}

/// How marker ink mixes with what is under it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ink {
    /// On a light background: multiply, so dark text stays dark.
    Darken,
    /// On a dark background, where multiplied ink would vanish: screen, so
    /// light text stays light.
    Lighten,
}

impl Ink {
    /// The ink that shows on most of `area` of an opaque RGBA picture `w`×`h`.
    pub fn over(rgba: &[u8], w: u32, h: u32, area: Rect) -> Self {
        let a = area.intersect(&Rect::new(0, 0, w as i32, h as i32));
        // A few thousand samples are plenty, however big the stroke.
        let step = (((a.w * a.h) as f32 / 4000.0).sqrt() as usize).max(1);
        let (mut dark, mut all) = (0u32, 0u32);
        for y in (a.y..a.bottom()).step_by(step) {
            for x in (a.x..a.right()).step_by(step) {
                let i = (y as usize * w as usize + x as usize) * 4;
                let [r, g, b] = [0, 1, 2].map(|c| u32::from(rgba[i + c]));
                dark += u32::from(luma(r, g, b) < 128);
                all += 1;
            }
        }
        // The majority decides: text covers less than its background.
        if dark * 2 > all {
            Self::Lighten
        } else {
            Self::Darken
        }
    }

    fn blend(self) -> BlendMode {
        match self {
            Self::Darken => BlendMode::Multiply,
            Self::Lighten => BlendMode::Screen,
        }
    }
}

/// Where a marker shape puts its ink, to judge what is under it.
pub fn ink_area(s: &Shape) -> Rect {
    match s.kind {
        // A straight stroke's points span no height of their own.
        Kind::Highlighter => s.bbox().expand((s.style.size / 2.0).ceil() as i32),
        _ => s.bbox(),
    }
}

/// Draws `s` onto the screen picture in `pm`; marker ink suits what it finds.
pub fn draw_shape(pm: &mut PixmapMut, s: &Shape, ts: Transform) {
    let ink = if matches!(s.kind, Kind::Highlighter | Kind::TextMark) {
        let a = ink_area(s);
        let area = Rect::new(
            (a.x as f32 * ts.sx + ts.tx).round() as i32,
            (a.y as f32 * ts.sy + ts.ty).round() as i32,
            (a.w as f32 * ts.sx).round() as i32,
            (a.h as f32 * ts.sy).round() as i32,
        );
        let (w, h) = (pm.width(), pm.height());
        Ink::over(pm.data_mut(), w, h, area)
    } else {
        Ink::Darken
    };
    draw_inked(pm, s, ts, ink);
}

/// Draws `s` with marker ink chosen by the caller (`pm` may be empty, with the
/// screen composited under it later).
pub fn draw_inked(pm: &mut PixmapMut, s: &Shape, ts: Transform, ink: Ink) {
    let paint = paint(s.style.color, 255);
    let w = s.style.size;
    match (&s.geom, s.kind) {
        (Geom::Box { .. }, Kind::Rect) => draw_rect(pm, s.bbox(), &paint, w, ts),
        (Geom::Box { .. }, Kind::Ellipse) => draw_ellipse(pm, s.bbox(), &paint, w, ts),
        (Geom::Box { .. }, Kind::TextMark) => {
            marker(pm, s.bbox(), s.style.color, 1.0, ink, ts);
        }
        (Geom::Box { .. }, _) => pixelate(pm, s.bbox(), w, ts),
        (Geom::Segment { a, b }, Kind::Arrow) => draw_arrow(pm, *a, *b, &paint, w, ts),
        (Geom::Segment { a, b }, _) => draw_line(pm, *a, *b, &paint, w, ts),
        (Geom::Stroke(st), Kind::Highlighter) => {
            draw_highlighter(pm, st.points(), paint, w, ink, ts);
        }
        (Geom::Stroke(st), _) => draw_pen(pm, st.points(), &paint, w, ts),
        (Geom::Anchor(_), _) => draw_text(pm, s, ts),
    }
}

fn draw_rect(pm: &mut PixmapMut, r: Rect, paint: &Paint, w: f32, ts: Transform) {
    if let Some(r) = r.to_skia() {
        let stroke = Stroke {
            line_join: LineJoin::Miter,
            ..round_stroke(w)
        };
        pm.stroke_path(&PathBuilder::from_rect(r), paint, &stroke, ts, None);
    }
}

fn draw_ellipse(pm: &mut PixmapMut, r: Rect, paint: &Paint, w: f32, ts: Transform) {
    if let Some(path) = r.to_skia().and_then(PathBuilder::from_oval) {
        pm.stroke_path(&path, paint, &round_stroke(w), ts, None);
    }
}

fn draw_line(pm: &mut PixmapMut, a: Point, b: Point, paint: &Paint, w: f32, ts: Transform) {
    if let Some(path) = polyline(&[a.to_f32(), b.to_f32()], false) {
        pm.stroke_path(&path, paint, &round_stroke(w), ts, None);
    }
}

fn draw_arrow(pm: &mut PixmapMut, a: Point, b: Point, paint: &Paint, w: f32, ts: Transform) {
    let Some((shaft_end, head)) = arrow_geometry(a, b, w) else {
        return;
    };
    let stroke = round_stroke(w);
    if let Some(path) = polyline(&[a.to_f32(), shaft_end], false) {
        pm.stroke_path(&path, paint, &stroke, ts, None);
    }
    if let Some(path) = polyline(&head, true) {
        pm.fill_path(&path, paint, FillRule::Winding, ts, None);
        // Round the head's corners slightly so it matches the stroke.
        let thin = Stroke {
            width: (w * 0.5).max(1.0),
            ..stroke
        };
        pm.stroke_path(&path, paint, &thin, ts, None);
    }
}

fn draw_pen(pm: &mut PixmapMut, pts: &[Point], paint: &Paint, w: f32, ts: Transform) {
    if let [only] = pts {
        let (x, y) = only.to_f32();
        if let Some(dot) = PathBuilder::from_circle(x, y, w / 2.0) {
            pm.fill_path(&dot, paint, FillRule::Winding, ts, None);
        }
    } else if let Some(path) = smooth_path(pts) {
        pm.stroke_path(&path, paint, &round_stroke(w), ts, None);
    }
}

/// Opacity of screened highlighter ink: strong enough to see on black, weak
/// enough that white text still stands out from it.
const LIGHTEN_ALPHA: f32 = 0.55;

fn draw_highlighter(
    pm: &mut PixmapMut,
    pts: &[Point],
    mut paint: Paint,
    w: f32,
    ink: Ink,
    ts: Transform,
) {
    // Either way the text under the marker stays readable.
    paint.blend_mode = ink.blend();
    if ink == Ink::Lighten {
        paint.shader.apply_opacity(LIGHTEN_ALPHA);
    }
    let path = match pts {
        // A dot: a zero-length path strokes to nothing, so nudge it.
        [only] => {
            let (x, y) = only.to_f32();
            polyline(&[(x, y), (x + 0.01, y)], false)
        }
        _ => smooth_path(pts),
    };
    if let Some(path) = path {
        let stroke = Stroke {
            line_cap: LineCap::Square,
            ..round_stroke(w)
        };
        pm.stroke_path(&path, &paint, &stroke, ts, None);
    }
}

/// How far a marker stroke over a line `h` tall may reach past its box.
pub fn marker_margin(h: i32) -> i32 {
    (h as f32 * (MARKER_OVERSHOOT + MARKER_WOBBLE)).ceil() as i32 + 2
}

/// A marker's ends run past the text, and its edges are not quite straight:
/// both as fractions of the line height.
const MARKER_OVERSHOOT: f32 = 0.24;
const MARKER_WOBBLE: f32 = 0.045;

/// Repeatable noise in -1..1, so a stroke looks the same every time it is
/// painted (on screen, while dragged, and in the saved file).
fn noise(seed: u32, i: u32) -> f32 {
    let mut x = seed.wrapping_mul(0x9E37_79B1) ^ i.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 13;
    (x % 2001) as f32 / 1000.0 - 1.0
}

/// A highlighter-pen stroke over `r` in colour `rgb`: translucent, with
/// slightly uneven edges and ends, ink pooled where the pen touched down, and
/// faint streaks along it. `strength` (0..1) fades it for previews. `how` it
/// blends keeps the text underneath readable.
pub fn marker(pm: &mut PixmapMut, r: Rect, rgb: u32, strength: f32, how: Ink, ts: Transform) {
    if r.w <= 0 || r.h <= 0 {
        return;
    }
    // Seeded by the size only: moving a stroke must not change its texture.
    let seed = (r.w as u32).wrapping_mul(31) ^ r.h as u32;
    let (x, y, w, h) = (r.x as f32, r.y as f32, r.w as f32, r.h as f32);
    let (over, wobble) = (h * MARKER_OVERSHOOT, h * MARKER_WOBBLE);
    // Screened ink shows at full colour on black, so it goes on thinner.
    let strength = match how {
        Ink::Darken => strength,
        Ink::Lighten => strength * 0.9,
    };
    let ink = |alpha: f32| {
        let mut p = paint(rgb, (alpha * strength).clamp(0.0, 255.0) as u8);
        p.blend_mode = how.blend();
        p
    };

    // Long, gently waving edges; blunt, slightly slanted ends.
    let steps = ((w / (h * 2.5).max(12.0)).ceil() as u32).clamp(2, 80);
    let edge = |base: f32, row: u32| -> Vec<(f32, f32)> {
        (0..=steps)
            .map(|i| {
                let at = x + w * i as f32 / steps as f32;
                (at, base + noise(seed, row + i) * wobble)
            })
            .collect()
    };
    let top = edge(y, 10);
    let mut bottom = edge(y + h, 500);
    bottom.reverse();
    let left = x - over * (0.75 + 0.25 * noise(seed, 1).abs());
    let right = x + w + over * (0.75 + 0.25 * noise(seed, 2).abs());

    let mut pb = PathBuilder::new();
    pb.move_to(left + over * 0.15, top[0].1 + h * 0.04);
    smooth_through(&mut pb, &top);
    // The end: nearly straight, bulging a little.
    pb.quad_to(right, y - wobble * 0.5, right - over * 0.1, y + h * 0.16);
    pb.quad_to(
        right + over * 0.12,
        y + h * 0.55,
        right - over * 0.25,
        y + h * 0.9,
    );
    pb.quad_to(right - over * 0.4, y + h + wobble, bottom[0].0, bottom[0].1);
    smooth_through(&mut pb, &bottom);
    // The start.
    pb.quad_to(
        left + over * 0.1,
        y + h + wobble * 0.6,
        left + over * 0.05,
        y + h * 0.8,
    );
    pb.quad_to(
        left - over * 0.08,
        y + h * 0.4,
        left + over * 0.15,
        top[0].1 + h * 0.04,
    );
    pb.close();
    let Some(body) = pb.finish() else {
        return;
    };
    pm.fill_path(&body, &ink(165.0), FillRule::Winding, ts, None);
    // Ink gathers along the edge.
    pm.stroke_path(
        &body,
        &ink(45.0),
        &round_stroke((h * 0.04).max(1.0)),
        ts,
        None,
    );

    // Streaks left by the felt tip.
    for (n, at) in [0.18_f32, 0.46, 0.64, 0.84].into_iter().enumerate() {
        let n = n as u32;
        let from = x - over * 0.3 + (noise(seed, 20 + n) + 1.0) * w * 0.08;
        let to = x + w + over * 0.3 - (noise(seed, 30 + n) + 1.0) * w * 0.1;
        let mid = y + h * (at + noise(seed, 40 + n) * 0.05);
        let streak = [
            (from, mid),
            ((from + to) / 2.0, mid + noise(seed, 50 + n) * wobble),
            (to, mid),
        ];
        if let Some(path) = polyline(&streak, false) {
            let width = h * (0.07 + 0.05 * (noise(seed, 60 + n) + 1.0));
            pm.stroke_path(&path, &ink(34.0), &round_stroke(width), ts, None);
        }
    }
    // Where the pen touched down, a little darker.
    let mut blot = PathBuilder::new();
    blot.move_to(left + over * 0.2, y + h * 0.08);
    blot.quad_to(x + h * 0.45, y + h * 0.2, x + h * 0.2, y + h * 0.55);
    blot.quad_to(x + h * 0.05, y + h * 0.95, left + over * 0.15, y + h * 0.86);
    blot.quad_to(left, y + h * 0.45, left + over * 0.2, y + h * 0.08);
    blot.close();
    if let Some(path) = blot.finish() {
        pm.fill_path(&path, &ink(34.0), FillRule::Winding, ts, None);
    }
}

/// Continues `pb` along `pts` with curves that pass near each point.
fn smooth_through(pb: &mut PathBuilder, pts: &[(f32, f32)]) {
    for w in pts.windows(2) {
        let mid = ((w[0].0 + w[1].0) / 2.0, (w[0].1 + w[1].1) / 2.0);
        pb.quad_to(w[0].0, w[0].1, mid.0, mid.1);
    }
    if let Some(&(x, y)) = pts.last() {
        pb.line_to(x, y);
    }
}

/// Straight segments through `pts` (closed into a polygon when `close`).
fn polyline(pts: &[(f32, f32)], close: bool) -> Option<tiny_skia::Path> {
    let (first, rest) = pts.split_first()?;
    let mut pb = PathBuilder::new();
    pb.move_to(first.0, first.1);
    for p in rest {
        pb.line_to(p.0, p.1);
    }
    if close {
        pb.close();
    }
    pb.finish()
}

/// Quadratic curves through segment midpoints smooth out mouse jitter.
fn smooth_path(pts: &[Point]) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    let first = pts.first()?.to_f32();
    pb.move_to(first.0, first.1);
    for pair in pts.windows(2).skip(1) {
        let (c, n) = (pair[0].to_f32(), pair[1].to_f32());
        pb.quad_to(c.0, c.1, f32::midpoint(c.0, n.0), f32::midpoint(c.1, n.1));
    }
    let last = pts.last()?.to_f32();
    pb.line_to(last.0, last.1);
    pb.finish()
}

/// Black or white, whichever reads better on `bg`.
pub fn contrast(bg: u32) -> u32 {
    let (r, g, b) = ((bg >> 16) & 0xFF, (bg >> 8) & 0xFF, bg & 0xFF);
    if luma(r, g, b) > 150 {
        0x000000
    } else {
        0xFFFFFF
    }
}

/// Perceived brightness (0..255) of a colour.
fn luma(r: u32, g: u32, b: u32) -> u32 {
    (299 * r + 587 * g + 114 * b) / 1000
}

fn draw_text(pm: &mut PixmapMut, s: &Shape, ts: Transform) {
    let size = s.style.size;
    let (px, py) = s.text_padding();
    let (x, y) = s.anchor().to_f32();
    let color = if s.style.bg {
        let r = size * 0.25;
        if let Some(rect) = s.bbox().to_skia() {
            let path = rounded_rect(rect, r);
            pm.fill_path(
                &path,
                &paint(s.style.color, 255),
                FillRule::Winding,
                ts,
                None,
            );
        }
        contrast(s.style.color)
    } else {
        s.style.color
    };
    s.text_layout()
        .draw(pm, x + px, y + py, size, &paint(color, 255), ts);
}

pub fn rounded_rect(r: tiny_skia::Rect, rad: f32) -> tiny_skia::Path {
    let (x, y, w, h) = (r.x(), r.y(), r.width(), r.height());
    let rad = rad.min(w / 2.0).min(h / 2.0);
    let mut pb = PathBuilder::new();
    pb.move_to(x + rad, y);
    pb.line_to(x + w - rad, y);
    pb.quad_to(x + w, y, x + w, y + rad);
    pb.line_to(x + w, y + h - rad);
    pb.quad_to(x + w, y + h, x + w - rad, y + h);
    pb.line_to(x + rad, y + h);
    pb.quad_to(x, y + h, x, y + h - rad);
    pb.line_to(x, y + rad);
    pb.quad_to(x, y, x + rad, y);
    pb.close();
    pb.finish().unwrap()
}

/// Replaces every `block`-sized cell of `r` with its average colour.
/// `ts` must be a scale + translate (no rotation), as used everywhere here.
fn pixelate(pm: &mut PixmapMut, r: Rect, block: f32, ts: Transform) {
    let (pw, ph) = (pm.width() as i32, pm.height() as i32);
    let x0 = (r.x as f32 * ts.sx + ts.tx).round() as i32;
    let y0 = (r.y as f32 * ts.sy + ts.ty).round() as i32;
    let x1 = (r.right() as f32 * ts.sx + ts.tx).round() as i32;
    let y1 = (r.bottom() as f32 * ts.sy + ts.ty).round() as i32;
    let bs = ((block * ts.sx).round() as i32).max(2);
    let data = pm.data_mut();
    let mut by = y0;
    while by < y1 {
        let mut bx = x0;
        while bx < x1 {
            let cell = Rect::new(bx, by, bs.min(x1 - bx), bs.min(y1 - by))
                .intersect(&Rect::new(0, 0, pw, ph));
            if !cell.is_empty() {
                let mut sum = [0u32; 4];
                for y in cell.y..cell.bottom() {
                    for x in cell.x..cell.right() {
                        let i = ((y * pw + x) * 4) as usize;
                        for c in 0..4 {
                            sum[c] += data[i + c] as u32;
                        }
                    }
                }
                let n = (cell.w * cell.h) as u32;
                let avg = sum.map(|v| (v / n) as u8);
                for y in cell.y..cell.bottom() {
                    let i = ((y * pw + cell.x) * 4) as usize;
                    for px in data[i..i + cell.w as usize * 4].as_chunks_mut::<4>().0 {
                        px.copy_from_slice(&avg);
                    }
                }
            }
            bx += bs;
        }
        by += bs;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marker_stroke_covers_its_line_stays_in_its_margin_and_repeats() {
        let r = Rect::new(40, 30, 200, 30);
        let draw = |r: Rect| {
            let mut pm = tiny_skia::Pixmap::new(300, 100).unwrap();
            pm.fill(tiny_skia::Color::WHITE);
            let ts = Transform::identity();
            marker(&mut pm.as_mut(), r, 0xFFCC00, 1.0, Ink::Darken, ts);
            pm
        };
        let pm = draw(r);
        assert_eq!(pm.data(), draw(r).data(), "same stroke every time");
        // The line is tinted yellow, not covered over.
        let mid = pm.pixel(140, 45).unwrap();
        assert!(mid.red() > 230 && mid.blue() < 180, "{mid:?}");
        // The ends run past the box, but never past the margin.
        assert!(pm.pixel(38, 48).unwrap().blue() < 250);
        let m = marker_margin(r.h);
        let outer = r.expand(m);
        for (i, p) in pm.pixels().iter().enumerate() {
            let at = Point::new((i % 300) as i32, (i / 300) as i32);
            if !outer.contains(at) {
                assert_eq!(p.blue(), 255, "paint outside the margin at {at:?}");
            }
        }
        // Black text under it stays black.
        let mut text = tiny_skia::Pixmap::new(300, 100).unwrap();
        text.fill(tiny_skia::Color::BLACK);
        let ts = Transform::identity();
        marker(&mut text.as_mut(), r, 0xFFCC00, 1.0, Ink::Darken, ts);
        assert_eq!(text.pixel(140, 45).unwrap().red(), 0);
    }

    #[test]
    fn a_mark_on_a_dark_background_shows_and_keeps_light_text() {
        let mut pm = tiny_skia::Pixmap::new(300, 100).unwrap();
        pm.fill(tiny_skia::Color::from_rgba8(20, 20, 20, 255));
        // A white "letter" inside the line.
        for y in 40..50 {
            for x in 100..110 {
                let i = (y * 300 + x) * 4;
                pm.data_mut()[i..i + 4].copy_from_slice(&[255; 4]);
            }
        }
        let style = Style {
            color: 0xFFCC00,
            size: 1.0,
            bg: false,
        };
        let mark = Shape::text_mark(Rect::new(40, 30, 200, 30), style);
        draw_shape(&mut pm.as_mut(), &mark, Transform::identity());
        let back = pm.pixel(200, 45).unwrap();
        assert!(back.red() > 90 && back.green() > 70, "{back:?}");
        assert!(back.red() < 190 && back.blue() < 40, "{back:?}");
        let letter = pm.pixel(105, 45).unwrap();
        assert!(letter.red() == 255 && letter.green() == 255, "{letter:?}");
    }

    #[test]
    fn ink_follows_most_of_the_background() {
        let mut data = [255u8, 255, 255, 255].repeat(100);
        let all = Rect::new(0, 0, 10, 10);
        assert_eq!(Ink::over(&data, 10, 10, all), Ink::Darken);
        data[..60 * 4].fill(0);
        assert_eq!(Ink::over(&data, 10, 10, all), Ink::Lighten);
        // Only the asked-for area counts, clipped to the picture.
        let light = Rect::new(0, 6, 30, 30);
        assert_eq!(Ink::over(&data, 10, 10, light), Ink::Darken);
        let outside = Rect::new(50, 50, 5, 5);
        assert_eq!(Ink::over(&data, 10, 10, outside), Ink::Darken);
    }

    /// `SHOTLY_MARKER_OUT=/path.png cargo test dump_marker -- --ignored`
    /// writes sample strokes to look at.
    #[test]
    #[ignore = "writes a preview file"]
    fn dump_marker() {
        let Some(path) = std::env::var_os("SHOTLY_MARKER_OUT") else {
            return;
        };
        let mut pm = tiny_skia::Pixmap::new(1800, 400).unwrap();
        pm.fill(tiny_skia::Color::WHITE);
        let dark = tiny_skia::Rect::from_xywh(900.0, 0.0, 900.0, 400.0).unwrap();
        pm.fill_rect(dark, &paint(0x000000, 255), Transform::identity(), None);
        for (i, (h, color)) in [
            (30, 0xFFCC00),
            (56, 0xFFCC00),
            (44, 0x34C759),
            (36, 0xFF3B30),
        ]
        .into_iter()
        .enumerate()
        {
            let y = 30 + 92 * i as i32;
            let size = h as f32 * 0.8;
            let l = crate::text::layout("Highlight the words in this line", size);
            for (x, text) in [(60, 0x111111), (960, 0xEEEEEE)] {
                l.draw(
                    &mut pm.as_mut(),
                    x as f32,
                    y as f32 + h as f32 * 0.05,
                    size,
                    &paint(text, 255),
                    Transform::identity(),
                );
                let style = Style {
                    color,
                    size: 1.0,
                    bg: false,
                };
                let mark = Shape::text_mark(Rect::new(x, y, l.w as i32, h), style);
                draw_shape(&mut pm.as_mut(), &mark, Transform::identity());
            }
        }
        pm.save_png(path).unwrap();
    }
    use crate::shape::Style;
    use tiny_skia::Pixmap;

    fn red_at(pm: &Pixmap, x: u32, y: u32) -> bool {
        let c = pm.pixel(x, y).unwrap();
        c.red() > 200 && c.green() < 50 && c.alpha() > 200
    }

    #[test]
    fn draws_each_kind_where_expected() {
        let style = Style {
            color: 0xFF0000,
            size: 4.0,
            bg: false,
        };
        let cases = [
            (Kind::Rect, (10, 10), (50, 50), (10, 30)),
            (Kind::Ellipse, (10, 10), (50, 50), (30, 10)),
            (Kind::Line, (10, 10), (50, 50), (30, 30)),
            (Kind::Arrow, (10, 30), (60, 30), (58, 30)),
            (Kind::Pen, (10, 10), (50, 10), (30, 10)),
        ];
        for (kind, a, b, probe) in cases {
            let mut pm = Pixmap::new(64, 64).unwrap();
            let mut s = Shape::new(kind, Point::new(a.0, a.1), style);
            s.drag_to(Point::new(b.0, b.1), false);
            draw_shape(&mut pm.as_mut(), &s, Transform::identity());
            assert!(
                red_at(&pm, probe.0, probe.1),
                "{kind:?} not drawn at {probe:?}"
            );
            assert!(!red_at(&pm, 1, 62), "{kind:?} drew outside");
        }
    }

    #[test]
    fn highlighter_multiplies_instead_of_covering() {
        let mut pm = Pixmap::new(40, 40).unwrap();
        pm.fill(tiny_skia::Color::from_rgba8(0, 0, 0, 255));
        for x in 20..40 {
            for y in 0..40 {
                let i = (y * 40 + x) * 4;
                pm.data_mut()[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
        let mut s = Shape::new(
            Kind::Highlighter,
            Point::new(5, 20),
            Style {
                color: 0xFFFF00,
                size: 10.0,
                bg: false,
            },
        );
        s.drag_to(Point::new(35, 20), false);
        draw_shape(&mut pm.as_mut(), &s, Transform::identity());
        let dark = pm.pixel(10, 20).unwrap();
        let light = pm.pixel(30, 20).unwrap();
        assert_eq!((dark.red(), dark.green(), dark.blue()), (0, 0, 0));
        assert_eq!((light.red(), light.green(), light.blue()), (255, 255, 0));
    }

    #[test]
    fn pixelate_averages_blocks() {
        let mut pm = Pixmap::new(20, 20).unwrap();
        for y in 0..20 {
            for x in 0..20 {
                let v = if (x + y) % 2 == 0 { 255 } else { 0 };
                let i = (y * 20 + x) * 4;
                pm.data_mut()[i..i + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        let mut s = Shape::new(
            Kind::Pixelate,
            Point::new(0, 0),
            Style {
                color: 0,
                size: 4.0,
                bg: false,
            },
        );
        s.drag_to(Point::new(8, 8), false);
        draw_shape(&mut pm.as_mut(), &s, Transform::identity());
        assert_eq!(pm.pixel(1, 1).unwrap().red(), 127);
        assert_eq!(pm.pixel(10, 10).unwrap().red(), 255);
    }

    #[test]
    fn text_draws_glyphs_and_background() {
        let mut pm = Pixmap::new(200, 60).unwrap();
        let mut s = Shape::new(
            Kind::Text,
            Point::new(0, 0),
            Style {
                color: 0x0000FF,
                size: 30.0,
                bg: true,
            },
        );
        s.text = "HHHH".into();
        draw_shape(&mut pm.as_mut(), &s, Transform::identity());
        let bg = pm.pixel(3, 20).unwrap();
        assert_eq!((bg.red(), bg.blue()), (0, 255));
        let white = (0..pm.width())
            .filter(|&x| pm.pixel(x, 25).unwrap().red() > 200)
            .count();
        assert!(white > 10, "expected white glyph pixels, got {white}");
    }

    #[test]
    fn transform_scales_for_export() {
        let style = Style {
            color: 0xFF0000,
            size: 2.0,
            bg: false,
        };
        let mut s = Shape::new(Kind::Line, Point::new(5, 10), style);
        s.drag_to(Point::new(25, 10), false);
        let mut pm = Pixmap::new(64, 64).unwrap();
        draw_shape(&mut pm.as_mut(), &s, Transform::from_scale(2.0, 2.0));
        assert!(red_at(&pm, 30, 20));
        assert!(!red_at(&pm, 15, 10));
    }
}
