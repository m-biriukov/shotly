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

pub fn draw_shape(pm: &mut PixmapMut, s: &Shape, ts: Transform) {
    let paint = paint(s.style.color, 255);
    let w = s.style.size;
    match (&s.geom, s.kind) {
        (Geom::Box { .. }, Kind::Rect) => draw_rect(pm, s.bbox(), &paint, w, ts),
        (Geom::Box { .. }, Kind::Ellipse) => draw_ellipse(pm, s.bbox(), &paint, w, ts),
        (Geom::Box { .. }, _) => pixelate(pm, s.bbox(), w, ts),
        (Geom::Segment { a, b }, Kind::Arrow) => draw_arrow(pm, *a, *b, &paint, w, ts),
        (Geom::Segment { a, b }, _) => draw_line(pm, *a, *b, &paint, w, ts),
        (Geom::Stroke(st), Kind::Highlighter) => draw_highlighter(pm, st.points(), paint, w, ts),
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

fn draw_highlighter(pm: &mut PixmapMut, pts: &[Point], mut paint: Paint, w: f32, ts: Transform) {
    // Multiply keeps dark text under the marker readable.
    paint.blend_mode = BlendMode::Multiply;
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
    let luma = (299 * r + 587 * g + 114 * b) / 1000;
    if luma > 150 { 0x000000 } else { 0xFFFFFF }
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
