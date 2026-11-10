//! Text layout and glyph outlines from the system fonts.
//! Glyphs become tiny-skia paths, so text is painted by the same code (and
//! the same transform) on screen and in the exported image.

use std::sync::OnceLock;

use skrifa::{DrawMemory, Font, GlyphId, PathSink, ScalerFont};
use tiny_skia::{FillRule, Paint, PathBuilder, PixmapMut, Transform};

/// (path, face index, variable-font weight), tried in order; later faces fill
/// in characters the earlier ones lack. A weight of ~600 reads better on busy
/// screenshots; static faces use their Medium cut instead.
const CANDIDATES: &[(&str, u32, Option<f32>)] = &[
    ("/System/Library/Fonts/SFNS.ttf", 0, Some(600.0)), // SF Pro, semibold-ish
    ("/System/Library/Fonts/HelveticaNeue.ttc", 10, None), // Helvetica Neue Medium
    ("/System/Library/Fonts/HelveticaNeue.ttc", 0, None),
    ("/System/Library/Fonts/Helvetica.ttc", 0, None),
    ("/Library/Fonts/Arial Unicode.ttf", 0, None),
    (
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        0,
        None,
    ),
];

/// One loaded face, drawn in font units (y up).
struct Face {
    scaler: ScalerFont,
    units_per_em: f32,
}

impl Face {
    fn font(&self) -> &Font {
        self.scaler.font()
    }

    /// Glyph for `c`, if the face maps it to something real.
    fn glyph_index(&self, c: char) -> Option<GlyphId> {
        self.font()
            .charmap()
            .map_unicode(c)
            .filter(|id| id.to_u32() != 0)
    }

    /// Advance in font units.
    fn advance(&self, id: GlyphId) -> f32 {
        self.font().glyph_metrics().h_advance(id)
    }

    /// (ascender, descender (negative), line gap) in font units.
    fn line(&self) -> (f32, f32, f32) {
        match self.font().metrics().scaled_f32(self.units_per_em).h_line() {
            Some(l) => (l.ascender, l.descender, l.line_gap),
            None => (self.units_per_em * 0.8, self.units_per_em * -0.2, 0.0),
        }
    }

    /// A face that cannot outline its own Latin capital is useless: skip it.
    fn can_outline(&self) -> bool {
        let Some(id) = self.glyph_index('H') else {
            return false;
        };
        let mut c = Counter(0);
        self.scaler
            .outline(id)
            .is_some_and(|o| o.draw(DrawMemory::Scratch, &mut c).is_ok())
            && c.0 > 4
    }
}

struct Fonts {
    faces: Vec<Face>,
}

fn fonts() -> &'static Fonts {
    static FONTS: OnceLock<Fonts> = OnceLock::new();
    FONTS.get_or_init(|| {
        let mut files: Vec<(&str, &'static [u8])> = Vec::new();
        let mut faces = Vec::new();
        for &(path, index, weight) in CANDIDATES {
            let data = match files.iter().find(|(p, _)| *p == path) {
                Some(&(_, d)) => d,
                None => {
                    let Ok(bytes) = std::fs::read(path) else {
                        continue;
                    };
                    let d: &'static [u8] = Box::leak(bytes.into_boxed_slice());
                    files.push((path, d));
                    d
                }
            };
            let Some(mut font) = Font::new(data, index) else {
                continue;
            };
            if let Some(w) = weight {
                font = font.instance_builder().variations([("wght", w)]).build();
            }
            let units_per_em = font.units_per_em() as f32;
            if units_per_em == 0.0 {
                continue;
            }
            let face = Face {
                scaler: ScalerFont::unhinted(font, None),
                units_per_em,
            };
            if face.can_outline() {
                faces.push(face);
            }
        }
        if faces.is_empty() {
            eprintln!("warning: no usable system font found; text will not render");
        }
        Fonts { faces }
    })
}

/// Counts path segments (used to check that a face really has outlines).
struct Counter(usize);

impl PathSink for Counter {
    fn move_to(&mut self, _: f32, _: f32) {}
    fn line_to(&mut self, _: f32, _: f32) {
        self.0 += 1;
    }
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {
        self.0 += 1;
    }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
        self.0 += 1;
    }
    fn close(&mut self) {}
}

struct Placed {
    face: usize,
    id: GlyphId,
    x: f32,
    line: usize,
}

/// Positions of every glyph of `text` at `size` px, top-left at (0, 0).
pub struct Layout {
    pub w: f32,
    pub h: f32,
    pub line_h: f32,
    ascent: f32,
    glyphs: Vec<Placed>,
    /// Caret position before each char, plus one after the last: (x, line).
    carets: Vec<(f32, usize)>,
}

pub fn layout(text: &str, size: f32) -> Layout {
    let f = fonts();
    let (ascent, line_h) = match f.faces.first() {
        Some(face) => {
            let s = size / face.units_per_em;
            let (a, d, gap) = face.line();
            let asc = a * s;
            (asc, (asc - d * s + gap * s).max(size))
        }
        None => (size, size * 1.2),
    };
    let mut glyphs = Vec::new();
    let mut carets = Vec::with_capacity(text.len() + 1);
    let (mut x, mut line, mut w) = (0.0f32, 0usize, 0.0f32);
    for c in text.chars() {
        carets.push((x, line));
        if c == '\n' {
            line += 1;
            x = 0.0;
            continue;
        }
        let found = f
            .faces
            .iter()
            .enumerate()
            .find_map(|(i, face)| face.glyph_index(c).map(|id| (i, id)));
        let Some((fi, id)) = found.or_else(|| f.faces.first().map(|_| (0, GlyphId::NOTDEF))) else {
            continue;
        };
        let face = &f.faces[fi];
        let adv = face.advance(id) * size / face.units_per_em;
        glyphs.push(Placed {
            face: fi,
            id,
            x,
            line,
        });
        x += adv;
        w = w.max(x);
    }
    carets.push((x, line));
    Layout {
        w,
        h: (line + 1) as f32 * line_h,
        line_h,
        ascent,
        glyphs,
        carets,
    }
}

impl Layout {
    /// Top of the caret before char `i`, relative to the text origin.
    pub fn caret(&self, i: usize) -> (f32, f32) {
        let (x, line) = self.carets[i.min(self.carets.len() - 1)];
        (x, line as f32 * self.line_h)
    }

    /// Fills the glyphs with their top-left at (`ox`, `oy`) in shape space.
    pub fn draw(
        &self,
        pm: &mut PixmapMut,
        ox: f32,
        oy: f32,
        size: f32,
        paint: &Paint,
        ts: Transform,
    ) {
        let f = fonts();
        let mut builder = Builder {
            pb: PathBuilder::new(),
            x: 0.0,
            y: 0.0,
            s: 1.0,
        };
        for g in &self.glyphs {
            let face = &f.faces[g.face];
            builder.s = size / face.units_per_em;
            builder.x = ox + g.x;
            builder.y = oy + g.line as f32 * self.line_h + self.ascent;
            if let Some(outline) = face.scaler.outline(g.id) {
                let _ = outline.draw(DrawMemory::Scratch, &mut builder);
            }
        }
        if let Some(path) = builder.pb.finish() {
            pm.fill_path(&path, paint, FillRule::Winding, ts, None);
        }
    }
}

/// Converts font units (y up) into shape-space pixels (y down).
struct Builder {
    pb: PathBuilder,
    x: f32,
    y: f32,
    s: f32,
}

impl Builder {
    fn p(&self, x: f32, y: f32) -> (f32, f32) {
        (self.x + x * self.s, self.y - y * self.s)
    }
}

impl PathSink for Builder {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.p(x, y);
        self.pb.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.p(x, y);
        self.pb.line_to(x, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (x1, y1) = self.p(x1, y1);
        let (x, y) = self.p(x, y);
        self.pb.quad_to(x1, y1, x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (x1, y1) = self.p(x1, y1);
        let (x2, y2) = self.p(x2, y2);
        let (x, y) = self.p(x, y);
        self.pb.cubic_to(x1, y1, x2, y2, x, y);
    }
    fn close(&mut self) {
        self.pb.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_grows_with_text_and_lines() {
        let one = layout("Hello", 20.0);
        let two = layout("Hello\nworld!", 20.0);
        assert!(one.w > 30.0, "width {}", one.w);
        assert!(two.h > one.h * 1.9);
        assert_eq!(layout("", 20.0).w, 0.0);
    }

    #[test]
    fn carets_step_through_chars_and_lines() {
        let l = layout("ab\nc", 20.0);
        assert_eq!(l.caret(0), (0.0, 0.0));
        assert!(l.caret(2).0 > l.caret(1).0);
        assert_eq!(l.caret(3), (0.0, l.line_h));
        assert!(l.caret(4).0 > 0.0);
    }

    #[test]
    fn primary_font_has_outlines() {
        let face = &fonts().faces[0];
        let mut c = Counter(0);
        let id = face.glyph_index('Ж').unwrap();
        face.scaler
            .outline(id)
            .unwrap()
            .draw(DrawMemory::Scratch, &mut c)
            .unwrap();
        assert!(c.0 > 4);
    }

    #[test]
    fn cyrillic_is_covered() {
        assert!(layout("Привет", 20.0).w > 30.0);
    }
}
