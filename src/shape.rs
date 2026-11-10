//! Annotation shapes: geometry, hit-testing and handle editing.
//! Coordinates are physical pixels of the overlay window.

use crate::geom::{Handle, Point, Rect, dist_to_segment, snap45, square};
use crate::text;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Rect,
    Ellipse,
    Line,
    Arrow,
    Pen,
    /// Wide translucent marker stroke (multiply blend).
    Highlighter,
    Text,
    /// Rectangle whose contents are pixelated.
    Pixelate,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    /// 0xRRGGBB
    pub color: u32,
    /// Size in window pixels; its meaning depends on the kind: stroke width,
    /// font size (text) or block size (pixelate). See `Width::pt`.
    pub size: f32,
    /// Text only: draw on a filled box of `color`.
    pub bg: bool,
}

/// A pen or highlighter stroke. Never empty: it starts with one point.
#[derive(Clone, Debug, PartialEq, Hash)]
pub struct Stroke(Vec<Point>);

impl Stroke {
    fn new(start: Point) -> Self {
        Self(vec![start])
    }

    pub fn points(&self) -> &[Point] {
        &self.0
    }

    fn first(&self) -> Point {
        self.0[0]
    }

    /// Adds `p` unless the mouse did not move.
    fn push(&mut self, p: Point) {
        if self.0.last() != Some(&p) {
            self.0.push(p);
        }
    }

    /// Replaces everything after the first point with a straight line to `p`.
    fn straight_to(&mut self, p: Point) {
        self.0.truncate(1);
        self.0.push(p);
    }
}

/// Where a shape is. Each kind has exactly one layout, so a shape cannot
/// hold, say, a rectangle with one corner.
#[derive(Clone, Debug, PartialEq, Hash)]
pub enum Geom {
    /// Rect, ellipse, pixelate: two opposite corners, in drawing order.
    Box { a: Point, b: Point },
    /// Line, arrow: from `a` to `b`.
    Segment { a: Point, b: Point },
    /// Pen, highlighter.
    Stroke(Stroke),
    /// Text: top-left corner of its box.
    Anchor(Point),
}

impl Geom {
    fn for_kind(kind: Kind, start: Point) -> Self {
        match kind {
            Kind::Rect | Kind::Ellipse | Kind::Pixelate => Self::Box { a: start, b: start },
            Kind::Line | Kind::Arrow => Self::Segment { a: start, b: start },
            Kind::Pen | Kind::Highlighter => Self::Stroke(Stroke::new(start)),
            Kind::Text => Self::Anchor(start),
        }
    }

    /// Applies `f` to every point.
    fn map_points(&mut self, mut f: impl FnMut(Point) -> Point) {
        match self {
            Self::Box { a, b } | Self::Segment { a, b } => {
                *a = f(*a);
                *b = f(*b);
            }
            Self::Stroke(s) => s.0.iter_mut().for_each(|p| *p = f(*p)),
            Self::Anchor(p) => *p = f(*p),
        }
    }

    /// Every point, in order.
    #[cfg(test)]
    pub fn points(&self) -> Vec<Point> {
        match self {
            Self::Box { a, b } | Self::Segment { a, b } => vec![*a, *b],
            Self::Stroke(s) => s.points().to_vec(),
            Self::Anchor(p) => vec![*p],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub kind: Kind,
    pub geom: Geom,
    pub style: Style,
    /// Text only.
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeHandle {
    /// One of the eight bounding-box handles (rect, ellipse, pen).
    Box(Handle),
    /// Line/arrow endpoints.
    Start,
    End,
}

/// Below this size (px) a freshly drawn shape counts as a click.
const MIN_SIZE: i32 = 3;

impl Shape {
    pub fn new(kind: Kind, start: Point, style: Style) -> Self {
        Self {
            kind,
            geom: Geom::for_kind(kind, start),
            style,
            text: String::new(),
        }
    }

    /// The point the shape was started at (the top-left corner for text).
    pub fn anchor(&self) -> Point {
        match &self.geom {
            Geom::Box { a, .. } | Geom::Segment { a, .. } => *a,
            Geom::Stroke(s) => s.first(),
            Geom::Anchor(p) => *p,
        }
    }

    /// Moves the whole shape so its anchor is at `p`.
    pub fn move_anchor_to(&mut self, p: Point) {
        let a = self.anchor();
        self.translate(p.x - a.x, p.y - a.y);
    }

    /// Text inset from the box edge, in px.
    pub fn text_padding(&self) -> (f32, f32) {
        (self.style.size * 0.3, self.style.size * 0.15)
    }

    pub fn text_layout(&self) -> text::Layout {
        text::layout(&self.text, self.style.size)
    }

    /// Extends the shape while it is being drawn. `shift` constrains
    /// rect/ellipse to square/circle, lines/arrows to 45° steps, and the
    /// highlighter to a straight stroke.
    pub fn drag_to(&mut self, p: Point, shift: bool) {
        match &mut self.geom {
            Geom::Box { a, b } => *b = if shift { square(*a, p) } else { p },
            Geom::Segment { a, b } => *b = if shift { snap45(*a, p) } else { p },
            Geom::Stroke(s) if shift && self.kind == Kind::Highlighter => s.straight_to(p),
            Geom::Stroke(s) => s.push(p),
            Geom::Anchor(_) => {}
        }
    }

    /// True when the drag was too small to mean anything (treated as a click).
    pub fn is_degenerate(&self) -> bool {
        let b = self.bbox();
        match self.geom {
            Geom::Box { .. } => b.w < MIN_SIZE || b.h < MIN_SIZE,
            Geom::Anchor(_) => false,
            Geom::Segment { .. } | Geom::Stroke(_) => b.w < MIN_SIZE && b.h < MIN_SIZE,
        }
    }

    pub fn bbox(&self) -> Rect {
        match &self.geom {
            Geom::Anchor(p) => {
                let l = self.text_layout();
                let (px, py) = self.text_padding();
                let w = l.w.max(self.style.size * 0.1) + 2.0 * px;
                let h = l.h + 2.0 * py;
                Rect::new(p.x, p.y, w.ceil() as i32, h.ceil() as i32)
            }
            Geom::Box { a, b } | Geom::Segment { a, b } => Rect::from_corners(*a, *b),
            Geom::Stroke(s) => {
                let pts = s.points();
                let l = pts.iter().map(|p| p.x).min().unwrap_or(0);
                let t = pts.iter().map(|p| p.y).min().unwrap_or(0);
                let r = pts.iter().map(|p| p.x).max().unwrap_or(0);
                let b = pts.iter().map(|p| p.y).max().unwrap_or(0);
                Rect::from_edges(l, t, r, b)
            }
        }
    }

    /// Is `p` within `tol` pixels of the shape's visible stroke?
    pub fn hit(&self, p: Point, tol: f32) -> bool {
        let tol = tol + self.style.size / 2.0;
        match (&self.geom, self.kind) {
            (Geom::Box { .. }, Kind::Rect) => dist_to_rect_outline(p, self.bbox()) <= tol,
            (Geom::Box { .. }, Kind::Ellipse) => dist_to_ellipse(p, self.bbox()) <= tol,
            // Pixelate and text are solid: anywhere inside counts.
            (Geom::Box { .. } | Geom::Anchor(_), _) => self.bbox().expand(tol as i32).contains(p),
            (Geom::Segment { a, b }, _) => dist_to_segment(p, *a, *b) <= tol,
            (Geom::Stroke(s), _) => match s.points() {
                [only] => dist_to_segment(p, *only, *only) <= tol,
                pts => pts
                    .windows(2)
                    .any(|w| dist_to_segment(p, w[0], w[1]) <= tol),
            },
        }
    }

    pub fn handles(&self) -> Vec<(ShapeHandle, Point)> {
        match &self.geom {
            Geom::Anchor(_) => Vec::new(),
            Geom::Segment { a, b } => vec![(ShapeHandle::Start, *a), (ShapeHandle::End, *b)],
            Geom::Box { .. } | Geom::Stroke(_) => {
                let b = self.bbox();
                Handle::ALL
                    .iter()
                    .map(|&h| (ShapeHandle::Box(h), h.anchor(&b)))
                    .collect()
            }
        }
    }

    pub fn translate(&mut self, dx: i32, dy: i32) {
        self.geom.map_points(|p| Point::new(p.x + dx, p.y + dy));
    }

    /// Sets this shape to `orig` edited by dragging `handle` from `start` to `p`.
    pub fn drag_handle(
        &mut self,
        orig: &Self,
        handle: ShapeHandle,
        start: Point,
        p: Point,
        shift: bool,
    ) {
        *self = orig.clone();
        match (handle, &mut self.geom) {
            (ShapeHandle::Start, Geom::Segment { a, b }) => {
                *a = if shift { snap45(*b, p) } else { p }
            }
            (ShapeHandle::End, Geom::Segment { a, b }) => {
                *b = if shift { snap45(*a, p) } else { p }
            }
            (ShapeHandle::Box(h), _) => {
                let ob = orig.bbox();
                let nb = h.apply(ob, p.x - start.x, p.y - start.y);
                let map = |v: i32, o0: i32, ow: i32, n0: i32, nw: i32| {
                    if ow == 0 {
                        n0
                    } else {
                        n0 + (i64::from(v - o0) * i64::from(nw) / i64::from(ow)) as i32
                    }
                };
                self.geom.map_points(|q| {
                    Point::new(
                        map(q.x, ob.x, ob.w, nb.x, nb.w),
                        map(q.y, ob.y, ob.h, nb.y, nb.h),
                    )
                });
            }
            // Endpoint handles only exist on segments.
            (ShapeHandle::Start | ShapeHandle::End, _) => {}
        }
    }
}

fn dist_to_rect_outline(p: Point, r: Rect) -> f32 {
    let (l, t, rt, b) = (r.x as f32, r.y as f32, r.right() as f32, r.bottom() as f32);
    let (x, y) = (p.x as f32, p.y as f32);
    if x >= l && x <= rt && y >= t && y <= b {
        (x - l).min(rt - x).min(y - t).min(b - y)
    } else {
        let dx = (l - x).max(0.0).max(x - rt);
        let dy = (t - y).max(0.0).max(y - b);
        dx.hypot(dy)
    }
}

/// Approximate distance to an ellipse outline, measured along the ray from its centre.
fn dist_to_ellipse(p: Point, r: Rect) -> f32 {
    let rx = r.w as f32 / 2.0;
    let ry = r.h as f32 / 2.0;
    if rx < 1.0 || ry < 1.0 {
        return dist_to_rect_outline(p, r);
    }
    let dx = p.x as f32 - (r.x as f32 + rx);
    let dy = p.y as f32 - (r.y as f32 + ry);
    let k = ((dx / rx).powi(2) + (dy / ry).powi(2)).sqrt();
    if k == 0.0 {
        return rx.min(ry);
    }
    dx.hypot(dy) * (k - 1.0).abs() / k
}

type Pt = (f32, f32);

/// Arrow layout for a shaft `a`→`b` of stroke width `w`: where the shaft
/// stops, and the head triangle (tip first). `None` for zero-length arrows.
pub fn arrow_geometry(a: Point, b: Point, w: f32) -> Option<(Pt, [Pt; 3])> {
    let (ax, ay, bx, by) = (a.x as f32, a.y as f32, b.x as f32, b.y as f32);
    let len = (bx - ax).hypot(by - ay);
    if len < 1.0 {
        return None;
    }
    let (ux, uy) = ((bx - ax) / len, (by - ay) / len);
    let head = (4.0 * w + 4.0).min(len * 0.7);
    let half = head * 0.5;
    let (base_x, base_y) = (bx - ux * head, by - uy * head);
    let shaft_end = (bx - ux * head * 0.8, by - uy * head * 0.8);
    let left = (base_x - uy * half, base_y + ux * half);
    let right = (base_x + uy * half, base_y - ux * half);
    Some((shaft_end, [(bx, by), left, right]))
}

#[cfg(test)]
mod tests {
    use super::*;

    const STYLE: Style = Style {
        color: 0xFF0000,
        size: 4.0,
        bg: false,
    };

    fn shape(kind: Kind, a: (i32, i32), b: (i32, i32)) -> Shape {
        let mut s = Shape::new(kind, Point::new(a.0, a.1), STYLE);
        s.drag_to(Point::new(b.0, b.1), false);
        s
    }

    #[test]
    fn rect_hit_on_outline_only() {
        let s = shape(Kind::Rect, (100, 100), (200, 200));
        assert!(s.hit(Point::new(100, 150), 3.0));
        assert!(s.hit(Point::new(96, 150), 3.0));
        assert!(!s.hit(Point::new(150, 150), 3.0));
        assert!(!s.hit(Point::new(80, 150), 3.0));
    }

    #[test]
    fn ellipse_hit_near_outline() {
        let s = shape(Kind::Ellipse, (0, 0), (200, 100));
        assert!(s.hit(Point::new(100, 1), 3.0));
        assert!(s.hit(Point::new(200, 50), 3.0));
        assert!(!s.hit(Point::new(100, 50), 3.0));
        assert!(!s.hit(Point::new(0, 0), 3.0));
    }

    #[test]
    fn pen_hit_along_stroke() {
        let mut s = Shape::new(Kind::Pen, Point::new(0, 0), STYLE);
        s.drag_to(Point::new(50, 0), false);
        s.drag_to(Point::new(50, 50), false);
        assert!(s.hit(Point::new(50, 25), 2.0));
        assert!(!s.hit(Point::new(25, 25), 2.0));
    }

    #[test]
    fn shift_makes_square_and_45_degree_line() {
        let mut r = Shape::new(Kind::Rect, Point::new(0, 0), STYLE);
        r.drag_to(Point::new(50, 20), true);
        assert_eq!(r.geom.points()[1], Point::new(50, 50));
        let mut l = Shape::new(Kind::Line, Point::new(0, 0), STYLE);
        l.drag_to(Point::new(100, 4), true);
        assert_eq!(l.geom.points()[1], Point::new(100, 0));
    }

    #[test]
    fn tiny_drag_is_degenerate() {
        assert!(shape(Kind::Rect, (10, 10), (11, 40)).is_degenerate());
        assert!(!shape(Kind::Line, (10, 10), (10, 40)).is_degenerate());
        assert!(Shape::new(Kind::Pen, Point::new(1, 1), STYLE).is_degenerate());
    }

    #[test]
    fn box_handle_scales_pen_points() {
        let mut s = Shape::new(Kind::Pen, Point::new(0, 0), STYLE);
        s.drag_to(Point::new(100, 50), false);
        s.drag_to(Point::new(100, 100), false);
        let orig = s.clone();
        s.drag_handle(
            &orig,
            ShapeHandle::Box(Handle::SE),
            Point::new(100, 100),
            Point::new(200, 150),
            false,
        );
        assert_eq!(
            s.geom.points(),
            vec![Point::new(0, 0), Point::new(200, 75), Point::new(200, 150)]
        );
    }

    #[test]
    fn endpoint_handle_moves_one_end() {
        let s = shape(Kind::Arrow, (0, 0), (100, 0));
        let mut e = s.clone();
        e.drag_handle(
            &s,
            ShapeHandle::Start,
            Point::new(0, 0),
            Point::new(30, 40),
            false,
        );
        assert_eq!(
            e.geom.points(),
            vec![Point::new(30, 40), Point::new(100, 0)]
        );
    }

    #[test]
    fn highlighter_shift_makes_straight_stroke() {
        let mut s = Shape::new(Kind::Highlighter, Point::new(0, 0), STYLE);
        s.drag_to(Point::new(10, 5), false);
        s.drag_to(Point::new(20, 3), false);
        s.drag_to(Point::new(100, 7), true);
        assert_eq!(s.geom.points(), vec![Point::new(0, 0), Point::new(100, 7)]);
    }

    #[test]
    fn text_bbox_follows_content_and_hits_inside() {
        let mut s = Shape::new(
            Kind::Text,
            Point::new(100, 100),
            Style {
                size: 20.0,
                ..STYLE
            },
        );
        let empty = s.bbox();
        s.text = "Hello world".into();
        let full = s.bbox();
        assert_eq!((full.x, full.y), (100, 100));
        assert!(full.w > empty.w + 50);
        assert!(s.hit(Point::new(full.x + full.w / 2, full.y + full.h / 2), 2.0));
        assert!(s.handles().is_empty());
    }

    #[test]
    fn pixelate_hits_anywhere_inside() {
        let s = shape(Kind::Pixelate, (0, 0), (100, 100));
        assert!(s.hit(Point::new(50, 50), 0.0));
        assert!(!s.hit(Point::new(150, 50), 0.0));
    }

    #[test]
    fn each_kind_gets_its_own_layout() {
        let p = Point::new(1, 2);
        assert!(matches!(
            Shape::new(Kind::Rect, p, STYLE).geom,
            Geom::Box { .. }
        ));
        assert!(matches!(
            Shape::new(Kind::Arrow, p, STYLE).geom,
            Geom::Segment { .. }
        ));
        assert!(matches!(
            Shape::new(Kind::Pen, p, STYLE).geom,
            Geom::Stroke(_)
        ));
        assert!(matches!(
            Shape::new(Kind::Text, p, STYLE).geom,
            Geom::Anchor(_)
        ));
        let mut t = Shape::new(Kind::Text, p, STYLE);
        t.move_anchor_to(Point::new(10, 20));
        assert_eq!(t.anchor(), Point::new(10, 20));
    }

    #[test]
    fn arrow_head_points_at_end() {
        let (shaft, head) = arrow_geometry(Point::new(0, 0), Point::new(100, 0), 4.0).unwrap();
        assert_eq!(head[0], (100.0, 0.0));
        assert!(shaft.0 < 100.0 && shaft.0 > 80.0);
        assert_eq!(head[1].0, head[2].0);
        assert!(arrow_geometry(Point::new(5, 5), Point::new(5, 5), 4.0).is_none());
    }
}
