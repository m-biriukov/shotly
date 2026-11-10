//! CPU rasterising for the overlay's layers: only the parts that change are
//! painted here; the frozen screen itself is a GPU layer (see `layers.rs`).

use image::RgbaImage;
use tiny_skia::{IntSize, Pixmap, Transform};

use crate::geom::{Rect, Scale};
use crate::paint;
use crate::shape::{Kind, Shape};
use crate::theme;

/// The frozen screen at window resolution, RGBA, for painting shapes onto
/// (highlighter and pixelate need the pixels underneath).
pub struct Backdrop {
    pub w: u32,
    pub h: u32,
    rgba: Vec<u8>,
}

impl Backdrop {
    /// Resamples `img` (nearest neighbour) to `w`×`h` if sizes differ.
    pub fn new(img: &RgbaImage, w: u32, h: u32) -> Self {
        let (iw, ih) = img.dimensions();
        let rgba = if (iw, ih) == (w, h) {
            img.as_raw().clone()
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
            v
        };
        Self { w, h, rgba }
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
            data.extend_from_slice(&self.rgba[s..s + row]);
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
/// redo on every mouse move. Returns its window rect, pixels, and whether the
/// layer must use multiply compositing (highlighter).
pub fn active(bd: &Backdrop, s: &Shape) -> Option<(Rect, Pixmap, bool)> {
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
    paint::draw_shape(&mut pm.as_mut(), s, ts);
    Some((r, pm, s.kind == Kind::Highlighter))
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
            &RgbaImage::from_pixel(400, 300, Rgba([200, 100, 50, 255])),
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
        let (r, pm, multiply) = active(&bd, &a).unwrap();
        assert!(!multiply);
        assert!(r.x < 50 && r.right() > 200 && r.y < 100 - 8 && r.bottom() > 100 + 8);
        // Transparent away from the stroke: composited over the screen layer.
        assert_eq!(pm.pixel(1, 1).unwrap().alpha(), 0);
        let tip = pm.pixel((199 - r.x) as u32, (100 - r.y) as u32).unwrap();
        assert!(tip.alpha() > 200);
    }

    #[test]
    fn highlighter_is_flagged_for_multiply_and_pixelate_uses_screen() {
        let bd = backdrop();
        let (_, _, multiply) =
            active(&bd, &line((50, 100), (200, 100), Kind::Highlighter)).unwrap();
        assert!(multiply);
        let (_, pm, _) = active(&bd, &line((50, 50), (150, 150), Kind::Pixelate)).unwrap();
        let p = pm.pixel(10, 10).unwrap();
        assert_eq!((p.red(), p.alpha()), (200, 255));
    }

    #[test]
    fn resamples_when_window_size_differs() {
        let mut img = RgbaImage::from_pixel(200, 160, Rgba([0, 0, 0, 255]));
        img.put_pixel(199, 159, Rgba([255, 255, 255, 255]));
        let bd = Backdrop::new(&img, 100, 80);
        assert_eq!((bd.w, bd.h, bd.rgba.len()), (100, 80, 100 * 80 * 4));
    }
}
