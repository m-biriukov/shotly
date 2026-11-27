//! CPU rasterising for the overlay's layers: only the parts that change are
//! painted here; the frozen screen itself is a GPU layer (see `layers.rs`).

use std::sync::Arc;

use image::RgbaImage;
use tiny_skia::{IntSize, Pixmap, Transform};

use crate::geom::{Rect, Scale};
use crate::paint::{self, Ink};
use crate::shape::{Kind, Shape};
use crate::theme;

/// The frozen screen at window resolution, RGBA, for painting shapes onto
/// (highlighter and pixelate need the pixels underneath).
pub struct Backdrop {
    pub w: u32,
    pub h: u32,
    /// Shared with the capture when the sizes match (the usual case), so the
    /// screen is not held twice.
    rgba: Arc<RgbaImage>,
}

impl Backdrop {
    /// Resamples `img` (nearest neighbour) to `w`×`h` if sizes differ.
    pub fn new(img: Arc<RgbaImage>, w: u32, h: u32) -> Self {
        let (iw, ih) = img.dimensions();
        let rgba = if (iw, ih) == (w, h) {
            img
        } else {
            let raw = img.as_raw();
            let mut v = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h {
                let sy = (y as u64 * ih as u64 / h as u64) as usize;
                for x in 0..w {
                    let sx = (x as u64 * iw as u64 / w as u64) as usize;
                    let i = (sy * iw as usize + sx) * 4;
                    v.extend_from_slice(&raw[i..i + 4]);
                }
            }
            Arc::new(RgbaImage::from_raw(w, h, v).expect("w × h pixels"))
        };
        Self { w, h, rgba }
    }

    fn bytes(&self) -> &[u8] {
        self.rgba.as_raw()
    }

    fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.w as i32, self.h as i32)
    }

    /// Copy of the screen inside `r` (which must lie within the backdrop).
    fn crop(&self, r: Rect) -> Pixmap {
        let row = r.w as usize * 4;
        let mut data = Vec::with_capacity(row * r.h as usize);
        for y in r.y..r.bottom() {
            let s = (y as usize * self.w as usize + r.x as usize) * 4;
            data.extend_from_slice(&self.bytes()[s..s + row]);
        }
        Pixmap::from_vec(data, IntSize::from_wh(r.w as u32, r.h as u32).unwrap()).unwrap()
    }
}

/// Area a shape can paint into: its box plus room for stroke width,
/// arrow heads and anti-aliasing.
pub fn paint_bounds(s: &Shape) -> Rect {
    let b = s.bbox();
    let w = s.style.size;
    let m = match s.kind {
        Kind::Arrow => 2.5 * w + 6.0,
        Kind::Text | Kind::Pixelate => 1.0,
        Kind::TextMark => paint::marker_margin(b.h) as f32,
        // Square caps reach out by half the width diagonally.
        Kind::Highlighter => 0.75 * w + 2.0,
        _ => 0.5 * w + 2.0,
    }
    .ceil() as i32;
    Rect::new(b.x - m, b.y - m, b.w + 2 * m, b.h + 2 * m)
}

/// The screen inside `r` with `shapes` painted on top, or `None` if `r` is
/// empty after clipping to the window.
pub fn region<'a>(
    bd: &Backdrop,
    r: Rect,
    shapes: impl Iterator<Item = &'a Shape>,
) -> Option<(Rect, Pixmap)> {
    let r = r.intersect(&bd.bounds());
    if r.w <= 0 || r.h <= 0 {
        return None;
    }
    let mut pm = bd.crop(r);
    let ts = Transform::from_translate(-r.x as f32, -r.y as f32);
    for s in shapes {
        paint::draw_shape(&mut pm.as_mut(), s, ts);
    }
    Some((r, pm))
}

/// The shape being drawn/dragged/typed, painted alone so it is cheap to
/// redo on every mouse move. Returns its window rect, pixels, and for marker
/// shapes the ink whose compositing the layer must use.
pub fn active(bd: &Backdrop, s: &Shape) -> Option<(Rect, Pixmap, Option<Ink>)> {
    let r = paint_bounds(s).intersect(&bd.bounds());
    if r.w <= 0 || r.h <= 0 {
        return None;
    }
    let ts = Transform::from_translate(-r.x as f32, -r.y as f32);
    // Pixelate needs the screen underneath; everything else draws on clear
    // pixels and is composited by Core Animation.
    let mut pm = if s.kind == Kind::Pixelate {
        bd.crop(r)
    } else {
        Pixmap::new(r.w as u32, r.h as u32)?
    };
    let ink = matches!(s.kind, Kind::Highlighter | Kind::TextMark)
        .then(|| Ink::over(bd.bytes(), bd.w, bd.h, paint::ink_area(s)));
    paint::draw_inked(&mut pm.as_mut(), s, ts, ink.unwrap_or(Ink::Darken));
    Some((r, pm, ink))
}

/// Side of the drawing cursor, in points; its hot spot is the middle.
pub const CROSSHAIR_SIDE: u16 = 24;

/// The drawing cursor: a crosshair in `rgb` with a thin outline that keeps it
/// visible on any background. Straight-alpha RGBA, one pixel per point.
pub fn crosshair(rgb: u32) -> Vec<u8> {
    let side = f32::from(CROSSHAIR_SIDE);
    let mid = side / 2.0;
    let mut pm = Pixmap::new(CROSSHAIR_SIDE.into(), CROSSHAIR_SIDE.into()).expect("non-empty");
    let mut bar = |along: f32, across: f32, paint: &tiny_skia::Paint| {
        let (from, length) = (along, side - 2.0 * along);
        for (x, y, w, h) in [
            (from, mid - across, length, 2.0 * across),
            (mid - across, from, 2.0 * across, length),
        ] {
            if let Some(r) = tiny_skia::Rect::from_xywh(x, y, w, h) {
                pm.fill_rect(r, paint, Transform::identity(), None);
            }
        }
    };
    bar(1.0, 2.0, &paint::paint(paint::contrast(rgb), 255));
    bar(2.0, 1.0, &paint::paint(rgb, 255));
    pm.pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect()
}

/// The preview over the selected words on a transparent picture covering
/// `bounds`: plain boxes while selecting, marker strokes while highlighting.
/// `None` if `bounds` is empty.
pub fn selection(
    bounds: Rect,
    rects: &[Rect],
    rgb: u32,
    alpha: u8,
    strokes: bool,
) -> Option<Pixmap> {
    let mut pm = Pixmap::new(bounds.w.max(0) as u32, bounds.h.max(0) as u32)?;
    let paint = paint::paint(rgb, alpha);
    for r in rects {
        let local = Rect::new(r.x - bounds.x, r.y - bounds.y, r.w, r.h);
        if strokes {
            let strength = f32::from(alpha) / 255.0;
            paint::marker(
                &mut pm.as_mut(),
                local,
                rgb,
                strength,
                // Over clear pixels the blend makes no difference.
                Ink::Darken,
                Transform::identity(),
            );
        } else if let Some(r) = local.to_skia() {
            pm.fill_rect(r, &paint, Transform::identity(), None);
        }
    }
    Some(pm)
}

/// "W × H" pill shown next to the selection.
pub fn size_label(w: i32, h: i32, scale: Scale) -> Pixmap {
    pill(&format!("{w} × {h}"), scale)
}

/// Small dark rounded label with light text (size label, tooltips).
pub fn pill(text: &str, scale: Scale) -> Pixmap {
    let s = scale.pxf(1.0);
    let size = 12.0 * s;
    let l = crate::text::layout(text, size);
    let (px, py) = (7.0 * s, 3.0 * s);
    let (bw, bh) = ((l.w + 2.0 * px).ceil(), (l.h + 2.0 * py).ceil());
    let mut pm = Pixmap::new(bw as u32, bh as u32).unwrap();
    let bg = paint::rounded_rect(
        tiny_skia::Rect::from_xywh(0.0, 0.0, bw, bh).unwrap(),
        5.0 * s,
    );
    pm.fill_path(
        &bg,
        &paint::paint(theme::PANEL, theme::PANEL_ALPHA),
        tiny_skia::FillRule::Winding,
        Transform::identity(),
        None,
    );
    l.draw(
        &mut pm.as_mut(),
        px,
        py,
        size,
        &paint::paint(theme::INK, 255),
        Transform::identity(),
    );
    pm
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Point;
    use crate::shape::Style;
    use image::Rgba;

    fn backdrop() -> Backdrop {
        Backdrop::new(
            Arc::new(RgbaImage::from_pixel(400, 300, Rgba([200, 100, 50, 255]))),
            400,
            300,
        )
    }

    fn line(a: (i32, i32), b: (i32, i32), kind: Kind) -> Shape {
        let mut s = Shape::new(
            kind,
            Point::new(a.0, a.1),
            Style {
                color: 0xFF0000,
                size: 4.0,
                bg: false,
            },
        );
        s.drag_to(Point::new(b.0, b.1), false);
        s
    }

    #[test]
    fn region_paints_shapes_over_screen_and_clips_to_window() {
        let bd = backdrop();
        let s = line((10, 50), (390, 50), Kind::Line);
        let (r, pm) = region(&bd, Rect::new(-20, 0, 100, 100), std::iter::once(&s)).unwrap();
        assert_eq!(r, Rect::new(0, 0, 80, 100));
        let on = pm.pixel(40, 50).unwrap();
        let off = pm.pixel(40, 10).unwrap();
        assert_eq!((on.red(), on.green()), (255, 0));
        assert_eq!((off.red(), off.green(), off.blue()), (200, 100, 50));
        assert!(region(&bd, Rect::new(500, 0, 10, 10), std::iter::empty()).is_none());
    }

    #[test]
    fn active_shape_is_painted_alone_with_room_for_arrow_head() {
        let bd = backdrop();
        let a = line((50, 100), (200, 100), Kind::Arrow);
        let (r, pm, ink) = active(&bd, &a).unwrap();
        assert_eq!(ink, None);
        assert!(r.x < 50 && r.right() > 200 && r.y < 100 - 8 && r.bottom() > 100 + 8);
        // Transparent away from the stroke: composited over the screen layer.
        assert_eq!(pm.pixel(1, 1).unwrap().alpha(), 0);
        let tip = pm.pixel((199 - r.x) as u32, (100 - r.y) as u32).unwrap();
        assert!(tip.alpha() > 200);
    }

    #[test]
    fn highlighter_gets_ink_for_its_background_and_pixelate_uses_screen() {
        let marker = line((50, 100), (200, 100), Kind::Highlighter);
        for (shade, expected) in [(255, Ink::Darken), (0, Ink::Lighten)] {
            let img = RgbaImage::from_pixel(400, 300, Rgba([shade, shade, shade, 255]));
            let (_, _, ink) = active(&Backdrop::new(Arc::new(img), 400, 300), &marker).unwrap();
            assert_eq!(ink, Some(expected));
        }
        let bd = backdrop();
        let (_, pm, _) = active(&bd, &line((50, 50), (150, 150), Kind::Pixelate)).unwrap();
        let p = pm.pixel(10, 10).unwrap();
        assert_eq!((p.red(), p.alpha()), (200, 255));
    }

    #[test]
    fn resamples_when_window_size_differs() {
        let mut img = RgbaImage::from_pixel(200, 160, Rgba([0, 0, 0, 255]));
        img.put_pixel(199, 159, Rgba([255, 255, 255, 255]));
        let bd = Backdrop::new(Arc::new(img), 100, 80);
        assert_eq!((bd.w, bd.h, bd.bytes().len()), (100, 80, 100 * 80 * 4));
        // The same size: shared, not copied.
        let img = Arc::new(RgbaImage::new(100, 80));
        let bd = Backdrop::new(Arc::clone(&img), 100, 80);
        assert!(Arc::ptr_eq(&img, &bd.rgba));
    }

    #[test]
    fn the_crosshair_has_its_colour_in_the_middle_and_an_outline() {
        let px = |img: &[u8], x: usize, y: usize| {
            let i = (y * usize::from(CROSSHAIR_SIDE) + x) * 4;
            [img[i], img[i + 1], img[i + 2], img[i + 3]]
        };
        let red = crosshair(0xFF3B30);
        assert_eq!(px(&red, 12, 12), [0xFF, 0x3B, 0x30, 255]);
        assert_eq!(px(&red, 5, 11), [0xFF, 0x3B, 0x30, 255], "left arm");
        assert_eq!(px(&red, 5, 10), [255, 255, 255, 255], "outline");
        assert_eq!(px(&red, 4, 4)[3], 0, "clear between the arms");
        // A light ink gets a dark outline.
        let yellow = crosshair(0xFFCC00);
        assert_eq!(px(&yellow, 5, 10), [0, 0, 0, 255]);
    }
}
