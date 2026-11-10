//! Pure geometry for the selection rectangle: creating, moving and resizing.
//! All coordinates are physical pixels of one overlay window.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    /// As drawing coordinates.
    pub fn to_f32(self) -> (f32, f32) {
        (self.x as f32, self.y as f32)
    }
}

/// A display's backing scale: how many physical pixels one point covers
/// (2.0 on Retina). Sizes in the UI are given in points and converted here,
/// so rounding is the same everywhere.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scale(pub f64);

impl Scale {
    /// Points → whole pixels.
    pub fn px(self, pt: f64) -> i32 {
        (pt * self.0).round() as i32
    }

    /// Points → fractional pixels, for drawing.
    pub fn pxf(self, pt: f32) -> f32 {
        pt * self.0 as f32
    }

    /// Pixels → points.
    pub fn pt(self, px: i32) -> f64 {
        f64::from(px) / self.0
    }

    /// Width of the thinnest crisp line: one point, at least one pixel.
    pub fn hairline(self) -> i32 {
        self.px(1.0).max(1)
    }

    pub fn get(self) -> f64 {
        self.0
    }
}

impl From<f64> for Scale {
    fn from(s: f64) -> Self {
        Self(s)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    /// Rectangle spanning two arbitrary corners.
    pub fn from_corners(a: Point, b: Point) -> Self {
        Self::from_edges(a.x, a.y, b.x, b.y)
    }

    /// Rectangle from left/top/right/bottom edges given in any order.
    pub fn from_edges(l: i32, t: i32, r: i32, b: i32) -> Self {
        let (l, r) = if l <= r { (l, r) } else { (r, l) };
        let (t, b) = if t <= b { (t, b) } else { (b, t) };
        Self::new(l, t, r - l, b - t)
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }

    pub fn clamp_point(&self, p: Point) -> Point {
        Point::new(
            p.x.clamp(self.x, self.right()),
            p.y.clamp(self.y, self.bottom()),
        )
    }

    /// Part of `self` that lies inside `other`.
    pub fn intersect(&self, other: &Self) -> Self {
        let l = self.x.max(other.x);
        let t = self.y.max(other.y);
        let r = self.right().min(other.right());
        let b = self.bottom().min(other.bottom());
        Self::new(l, t, (r - l).max(0), (b - t).max(0))
    }

    /// Smallest rectangle covering both.
    pub fn union(&self, other: &Self) -> Self {
        Self::from_edges(
            self.x.min(other.x),
            self.y.min(other.y),
            self.right().max(other.right()),
            self.bottom().max(other.bottom()),
        )
    }

    /// `self` grown by `by` on every side.
    pub fn expand(&self, by: i32) -> Self {
        Self::new(self.x - by, self.y - by, self.w + 2 * by, self.h + 2 * by)
    }

    /// Square of half-size `half` centred on `c` (handles, markers).
    pub fn around(c: Point, half: i32) -> Self {
        Self::new(c.x - half, c.y - half, 2 * half, 2 * half)
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    /// Left edge for a `width`-wide box centred over `self`, kept at least
    /// `margin` inside `within` (toolbar, flyouts, tooltips).
    pub fn centered_x(&self, width: i32, within: &Self, margin: i32) -> i32 {
        let min = within.x + margin;
        (self.x + self.w / 2 - width / 2).clamp(min, (within.right() - width - margin).max(min))
    }

    /// As a drawing rectangle; `None` when empty.
    pub fn to_skia(self) -> Option<tiny_skia::Rect> {
        tiny_skia::Rect::from_xywh(self.x as f32, self.y as f32, self.w as f32, self.h as f32)
    }
}

/// `p` moved so the line `origin`→`p` sits on the nearest multiple of 45°.
pub fn snap45(origin: Point, p: Point) -> Point {
    let (dx, dy) = ((p.x - origin.x) as f64, (p.y - origin.y) as f64);
    let len = dx.hypot(dy);
    let step = std::f64::consts::FRAC_PI_4;
    let angle = (dy.atan2(dx) / step).round() * step;
    Point::new(
        origin.x + (len * angle.cos()).round() as i32,
        origin.y + (len * angle.sin()).round() as i32,
    )
}

/// `p` moved so `origin`→`p` spans a square (keeps the drag direction).
pub fn square(origin: Point, p: Point) -> Point {
    let (dx, dy) = (p.x - origin.x, p.y - origin.y);
    let side = dx.abs().max(dy.abs());
    Point::new(origin.x + side * dx.signum(), origin.y + side * dy.signum())
}

/// Distance from `p` to the segment `a`–`b`.
pub fn dist_to_segment(p: Point, a: Point, b: Point) -> f32 {
    let (px, py) = (p.x as f32, p.y as f32);
    let (ax, ay) = (a.x as f32, a.y as f32);
    let (dx, dy) = (b.x as f32 - ax, b.y as f32 - ay);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0)
    };
    (px - (ax + t * dx)).hypot(py - (ay + t * dy))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
}

impl Handle {
    pub const ALL: [Self; 8] = [
        Self::NW,
        Self::N,
        Self::NE,
        Self::E,
        Self::SE,
        Self::S,
        Self::SW,
        Self::W,
    ];

    /// Centre of this handle on `r`.
    pub fn anchor(self, r: &Rect) -> Point {
        let cx = r.x + r.w / 2;
        let cy = r.y + r.h / 2;
        match self {
            Self::NW => Point::new(r.x, r.y),
            Self::N => Point::new(cx, r.y),
            Self::NE => Point::new(r.right(), r.y),
            Self::E => Point::new(r.right(), cy),
            Self::SE => Point::new(r.right(), r.bottom()),
            Self::S => Point::new(cx, r.bottom()),
            Self::SW => Point::new(r.x, r.bottom()),
            Self::W => Point::new(r.x, cy),
        }
    }

    /// `orig` with this handle's edges shifted by (`dx`, `dy`); flips if dragged past
    /// the opposite edge.
    pub fn apply(self, orig: Rect, dx: i32, dy: i32) -> Rect {
        let mut l = orig.x;
        let mut t = orig.y;
        let mut r = orig.right();
        let mut b = orig.bottom();
        if self.moves_left() {
            l += dx;
        }
        if self.moves_right() {
            r += dx;
        }
        if self.moves_top() {
            t += dy;
        }
        if self.moves_bottom() {
            b += dy;
        }
        Rect::from_edges(l, t, r, b)
    }

    fn moves_left(self) -> bool {
        matches!(self, Self::W | Self::NW | Self::SW)
    }
    fn moves_right(self) -> bool {
        matches!(self, Self::E | Self::NE | Self::SE)
    }
    fn moves_top(self) -> bool {
        matches!(self, Self::N | Self::NE | Self::NW)
    }
    fn moves_bottom(self) -> bool {
        matches!(self, Self::S | Self::SE | Self::SW)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Handle(Handle),
    Inside,
    Outside,
}

#[derive(Clone, Copy, Debug)]
enum Drag {
    Creating {
        anchor: Point,
    },
    Moving {
        start: Point,
        orig: Rect,
    },
    Resizing {
        handle: Handle,
        start: Point,
        orig: Rect,
    },
}

/// Drags smaller than this (in either direction) count as a click, not a selection.
const MIN_SIZE: i32 = 3;

/// Selection state for one overlay window.
pub struct Selection {
    bounds: Rect,
    /// How far from a handle's centre a press still grabs it.
    grab_radius: i32,
    rect: Option<Rect>,
    drag: Option<Drag>,
}

impl Selection {
    pub fn new(bounds: Rect, grab_radius: i32) -> Self {
        Self {
            bounds,
            grab_radius,
            rect: None,
            drag: None,
        }
    }

    pub fn rect(&self) -> Option<Rect> {
        self.rect
    }

    /// Sets the selection directly (e.g. to a clicked window's frame).
    pub fn set_rect(&mut self, r: Rect) {
        self.rect = Some(r.intersect(&self.bounds));
        self.drag = None;
    }

    pub fn clear(&mut self) {
        self.rect = None;
        self.drag = None;
    }

    pub fn set_bounds(&mut self, bounds: Rect) {
        self.bounds = bounds;
        self.rect = self.rect.map(|r| r.intersect(&bounds));
    }

    pub fn hit(&self, p: Point) -> Hit {
        let Some(r) = self.rect else {
            return Hit::Outside;
        };
        let g = self.grab_radius;
        for h in Handle::ALL {
            let a = h.anchor(&r);
            if (p.x - a.x).abs() <= g && (p.y - a.y).abs() <= g {
                return Hit::Handle(h);
            }
        }
        if r.contains(p) {
            Hit::Inside
        } else {
            Hit::Outside
        }
    }

    pub fn press(&mut self, p: Point) {
        let p = self.bounds.clamp_point(p);
        self.drag = Some(match (self.hit(p), self.rect) {
            (Hit::Handle(handle), Some(orig)) => Drag::Resizing {
                handle,
                start: p,
                orig,
            },
            (Hit::Inside, Some(orig)) => Drag::Moving { start: p, orig },
            _ => Drag::Creating { anchor: p },
        });
    }

    /// Returns true when the selection changed and needs a redraw.
    pub fn motion(&mut self, p: Point) -> bool {
        let Some(drag) = self.drag else { return false };
        let p = self.bounds.clamp_point(p);
        let b = self.bounds;
        let new = match drag {
            Drag::Creating { anchor } => Rect::from_corners(anchor, p),
            Drag::Moving { start, orig } => {
                let x = (orig.x + p.x - start.x).clamp(b.x, b.right() - orig.w);
                let y = (orig.y + p.y - start.y).clamp(b.y, b.bottom() - orig.h);
                Rect::new(x, y, orig.w, orig.h)
            }
            Drag::Resizing {
                handle,
                start,
                orig,
            } => handle
                .apply(orig, p.x - start.x, p.y - start.y)
                .intersect(&b),
        };
        let changed = self.rect != Some(new);
        self.rect = Some(new);
        changed
    }

    pub fn release(&mut self, p: Point) {
        self.motion(p);
        if let Some(Drag::Creating { .. }) = self.drag
            && self.rect.is_some_and(|r| r.w < MIN_SIZE || r.h < MIN_SIZE)
        {
            self.rect = None;
        }
        self.drag = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel() -> Selection {
        Selection::new(Rect::new(0, 0, 1000, 800), 10)
    }

    fn drag(s: &mut Selection, from: (i32, i32), to: (i32, i32)) {
        s.press(Point::new(from.0, from.1));
        s.motion(Point::new(to.0, to.1));
        s.release(Point::new(to.0, to.1));
    }

    #[test]
    fn creates_rect_from_any_drag_direction() {
        let mut s = sel();
        drag(&mut s, (300, 400), (100, 150));
        assert_eq!(s.rect(), Some(Rect::new(100, 150, 200, 250)));
    }

    #[test]
    fn creation_is_clamped_to_bounds() {
        let mut s = sel();
        drag(&mut s, (900, 700), (1500, -50));
        assert_eq!(s.rect(), Some(Rect::new(900, 0, 100, 700)));
    }

    #[test]
    fn click_without_drag_makes_no_selection() {
        let mut s = sel();
        drag(&mut s, (50, 50), (51, 51));
        assert_eq!(s.rect(), None);
    }

    #[test]
    fn dragging_inside_moves_and_stays_in_bounds() {
        let mut s = sel();
        drag(&mut s, (100, 100), (300, 200));
        drag(&mut s, (200, 150), (250, 170));
        assert_eq!(s.rect(), Some(Rect::new(150, 120, 200, 100)));
        drag(&mut s, (200, 150), (5000, 5000));
        assert_eq!(s.rect(), Some(Rect::new(800, 700, 200, 100)));
    }

    #[test]
    fn handle_resizes_one_edge() {
        let mut s = sel();
        drag(&mut s, (100, 100), (300, 200));
        assert_eq!(s.hit(Point::new(305, 150)), Hit::Handle(Handle::E));
        drag(&mut s, (300, 150), (400, 999));
        assert_eq!(s.rect(), Some(Rect::new(100, 100, 300, 100)));
    }

    #[test]
    fn corner_resize_past_opposite_corner_flips() {
        let mut s = sel();
        drag(&mut s, (100, 100), (300, 200));
        drag(&mut s, (300, 200), (50, 60));
        assert_eq!(s.rect(), Some(Rect::new(50, 60, 50, 40)));
    }

    #[test]
    fn snap45_picks_nearest_octant() {
        let o = Point::new(0, 0);
        assert_eq!(snap45(o, Point::new(100, 10)), Point::new(100, 0));
        assert_eq!(snap45(o, Point::new(100, 90)), Point::new(95, 95));
        assert_eq!(snap45(o, Point::new(-5, -100)), Point::new(0, -100));
    }

    #[test]
    fn square_keeps_direction() {
        let o = Point::new(10, 10);
        assert_eq!(square(o, Point::new(40, 0)), Point::new(40, -20));
    }

    #[test]
    fn segment_distance() {
        let (a, b) = (Point::new(0, 0), Point::new(10, 0));
        assert_eq!(dist_to_segment(Point::new(5, 3), a, b), 3.0);
        assert_eq!(dist_to_segment(Point::new(13, 4), a, b), 5.0);
    }

    #[test]
    fn rect_helpers() {
        let a = Rect::new(10, 10, 20, 20);
        assert_eq!(a.union(&Rect::new(50, 0, 5, 5)), Rect::new(10, 0, 45, 30));
        assert_eq!(a.expand(2), Rect::new(8, 8, 24, 24));
        assert_eq!(Rect::around(Point::new(5, 5), 3), Rect::new(2, 2, 6, 6));
        let screen = Rect::new(0, 0, 100, 100);
        assert_eq!(Rect::new(40, 0, 20, 10).centered_x(30, &screen, 5), 35);
        assert_eq!(Rect::new(0, 0, 4, 4).centered_x(30, &screen, 5), 5);
        assert_eq!(Rect::new(96, 0, 4, 4).centered_x(30, &screen, 5), 65);
        assert!(Rect::new(0, 0, 0, 5).is_empty());
    }

    #[test]
    fn scale_converts_points() {
        let s = Scale(2.0);
        assert_eq!((s.px(4.5), s.pt(9), s.hairline()), (9, 4.5, 2));
        assert_eq!(Scale(1.0).hairline(), 1);
    }

    #[test]
    fn press_outside_starts_new_selection() {
        let mut s = sel();
        drag(&mut s, (100, 100), (300, 200));
        assert_eq!(s.hit(Point::new(600, 600)), Hit::Outside);
        drag(&mut s, (600, 600), (700, 650));
        assert_eq!(s.rect(), Some(Rect::new(600, 600, 100, 50)));
    }
}
