//! "Copied" / "Saved" notice: slides in at a screen corner, waits, then
//! slides out. Size, corner and distance from the screen edge come from
//! Settings.

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::Result;
use tiny_skia::{FillRule, Pixmap, Transform};
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

use crate::config::{Config, ToastCorner, ToastOffset, ToastSize};
use crate::macos::{self, Frame};
use crate::{paint, text, theme};

// Sizes at "Small" (factor 1.0), in points; the chosen size scales all of them.
const TITLE_PT: f32 = 13.0;
const DETAIL_PT: f32 = 11.0;
const PAD_PT: f32 = 12.0;
const GAP_PT: f32 = 3.0;
const MAX_W_PT: f32 = 380.0;
const RADIUS_PT: f32 = 10.0;
/// How far the notice travels while sliding in and out.
const SLIDE_PT: f32 = 48.0;

const ENTER: Duration = Duration::from_millis(300);
const EXIT: Duration = Duration::from_millis(220);
/// Animation frame interval (~60 fps).
const FRAME: Duration = Duration::from_millis(16);

/// What a toast says and does.
pub struct Notice {
    pub title: String,
    pub detail: Option<String>,
    pub on_click: Option<OnClick>,
    /// How long it stays fully visible before sliding out.
    pub duration: Duration,
    /// Shown even when notices are turned off (problems the user must fix).
    pub important: bool,
}

impl Notice {
    pub fn new(title: impl Into<String>, duration: Duration) -> Self {
        Self {
            title: title.into(),
            detail: None,
            on_click: None,
            duration,
            important: false,
        }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn on_click(mut self, action: OnClick) -> Self {
        self.on_click = Some(action);
        self
    }

    pub fn important(mut self) -> Self {
        self.important = true;
        self
    }
}

/// What clicking the toast does.
#[derive(Clone, Debug)]
pub enum OnClick {
    /// Show the saved file in Finder.
    Reveal(PathBuf),
    /// Open System Settings › Screen Recording.
    PermissionSettings,
}

/// Where, how big and how far from the edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToastStyle {
    pub corner: ToastCorner,
    pub size: ToastSize,
    pub offset: ToastOffset,
}

impl From<&Config> for ToastStyle {
    fn from(c: &Config) -> Self {
        Self {
            corner: c.toast_position,
            size: c.toast_size,
            offset: c.toast_offset,
        }
    }
}

/// Where the notice is at a moment of its life.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Pose {
    /// 0 = invisible, 1 = opaque.
    alpha: f32,
    /// 1 = fully slid out towards the screen edge, 0 = in place.
    slide: f32,
    /// Still moving (needs frequent updates).
    animating: bool,
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

fn ease_in(t: f32) -> f32 {
    t * t * t
}

/// The notice `t` after it appeared, or `None` once it is gone. It slides in
/// over `ENTER`, stays until `hold`, then slides out over `EXIT`. Without
/// motion (Reduce Motion) it just appears and disappears.
fn pose(t: Duration, hold: Duration, motion: bool) -> Option<Pose> {
    let still = Pose {
        alpha: 1.0,
        slide: 0.0,
        animating: false,
    };
    if !motion {
        return (t < hold).then_some(still);
    }
    if t < ENTER {
        let p = ease_out(t.as_secs_f32() / ENTER.as_secs_f32());
        Some(Pose {
            alpha: p,
            slide: 1.0 - p,
            animating: true,
        })
    } else if t < hold {
        Some(still)
    } else if t < hold + EXIT {
        let q = ease_in((t - hold).as_secs_f32() / EXIT.as_secs_f32());
        // Leaves a little less far than it arrived: quick and unobtrusive.
        Some(Pose {
            alpha: 1.0 - q,
            slide: 0.5 * q,
            animating: true,
        })
    } else {
        None
    }
}

/// Top-left of a `size`-point notice in `corner` of `area`, `offset` points
/// from the edges.
fn place(area: Frame, corner: ToastCorner, size: (f32, f32), offset: i32) -> (i32, i32) {
    let (ax, ay, aw, ah) = area;
    let (w, h) = (size.0.ceil() as i32, size.1.ceil() as i32);
    let x = if corner.is_right() {
        ax + aw as i32 - w - offset
    } else {
        ax + offset
    };
    let y = if corner.is_top() {
        ay + offset
    } else {
        ay + ah as i32 - h - offset
    };
    (x, y)
}

pub struct Toast {
    window: Rc<Window>,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    title: String,
    detail: Option<String>,
    /// Size factor (see `ToastSize::factor`).
    k: f32,
    on_click: Option<OnClick>,
    /// Final position in points.
    at: (i32, i32),
    /// +1 slides in from the right edge, -1 from the left.
    dir: i32,
    started: Instant,
    hold: Duration,
    motion: bool,
    shown: Option<Pose>,
}

/// What the app should do with a toast after `Toast::step`.
pub enum Step {
    /// Come back at this time.
    Wait(Instant),
    /// Gone: drop it.
    Done,
}

impl Toast {
    /// `display` is the frame (global points) of the display to show it on.
    pub fn show(
        el: &ActiveEventLoop,
        display: Frame,
        style: ToastStyle,
        notice: Notice,
    ) -> Result<Self> {
        let k = style.size.factor();
        let detail = notice
            .detail
            .map(|d| ellipsize(&d, DETAIL_PT * k, (MAX_W_PT - 2.0 * PAD_PT) * k));
        let (w, h) = size_pt(&notice.title, detail.as_deref(), k);
        let at = place(
            macos::usable_area(display),
            style.corner,
            (w, h),
            style.offset.pt(),
        );
        let attrs = Window::default_attributes()
            .with_title("shotly")
            .with_decorations(false)
            .with_resizable(false)
            .with_active(false)
            .with_visible(false)
            .with_transparent(true)
            .with_position(LogicalPosition::new(at.0, at.1))
            .with_inner_size(LogicalSize::new(f64::from(w.ceil()), f64::from(h.ceil())));
        let window = Rc::new(el.create_window(attrs)?);
        macos::style_toast(&window, f64::from(RADIUS_PT * k));
        let context =
            softbuffer::Context::new(window.clone()).map_err(|e| anyhow::anyhow!("{e}"))?;
        let surface = softbuffer::Surface::new(&context, window.clone())
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut toast = Self {
            window,
            surface,
            title: notice.title,
            detail,
            k,
            on_click: notice.on_click,
            at,
            dir: if style.corner.is_right() { 1 } else { -1 },
            started: Instant::now(),
            hold: notice.duration,
            motion: !macos::reduce_motion(),
            shown: None,
        };
        // Place it at its starting pose (transparent, slid out) before it is
        // visible, so it never flashes at the final spot.
        toast.step(toast.started);
        toast.window.set_visible(true);
        toast.window.request_redraw();
        Ok(toast)
    }

    pub fn owns(&self, id: WindowId) -> bool {
        self.window.id() == id
    }

    pub fn on_click(&self) -> Option<&OnClick> {
        self.on_click.as_ref()
    }

    /// Moves/fades the window to where it should be at `now`.
    pub fn step(&mut self, now: Instant) -> Step {
        let t = now.saturating_duration_since(self.started);
        let Some(p) = pose(t, self.hold, self.motion) else {
            return Step::Done;
        };
        if self.shown != Some(p) {
            let dx = (p.slide * SLIDE_PT * self.k).round() as i32 * self.dir;
            self.window
                .set_outer_position(LogicalPosition::new(self.at.0 + dx, self.at.1));
            macos::set_window_alpha(&self.window, f64::from(p.alpha));
            self.shown = Some(p);
        }
        Step::Wait(if p.animating {
            now + FRAME
        } else {
            self.started + self.hold
        })
    }

    pub fn redraw(&mut self) {
        let size = self.window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else {
            return;
        };
        let s = self.window.scale_factor() as f32;
        let pm = draw(
            &self.title,
            self.detail.as_deref(),
            w.get(),
            h.get(),
            s,
            self.k,
        );
        if self.surface.resize(w, h).is_err() {
            return;
        }
        let Ok(mut buf) = self.surface.buffer_mut() else {
            return;
        };
        for (d, p) in buf.iter_mut().zip(pm.data().as_chunks::<4>().0) {
            *d = (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32;
        }
        let _ = buf.present();
    }
}

/// Size in points at size factor `k`.
fn size_pt(title: &str, detail: Option<&str>, k: f32) -> (f32, f32) {
    let t = text::layout(title, TITLE_PT * k);
    let mut w = t.w;
    let mut h = t.h;
    if let Some(d) = detail {
        let d = text::layout(d, DETAIL_PT * k);
        w = w.max(d.w);
        h += GAP_PT * k + d.h;
    }
    (
        (w + 2.0 * PAD_PT * k).ceil().min(MAX_W_PT * k),
        (h + 2.0 * PAD_PT * 0.8 * k).ceil(),
    )
}

/// Shortens `s` with a middle "…" until it fits in `max_w` points.
fn ellipsize(s: &str, size: f32, max_w: f32) -> String {
    if text::layout(s, size).w <= max_w {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut keep = chars.len();
    while keep > 4 {
        keep -= 1;
        let head: String = chars[..keep / 2].iter().collect();
        let tail: String = chars[chars.len() - (keep - keep / 2)..].iter().collect();
        let candidate = format!("{head}…{tail}");
        if text::layout(&candidate, size).w <= max_w {
            return candidate;
        }
    }
    s.chars().take(4).collect::<String>() + "…"
}

/// Draws the notice at backing scale `s` and size factor `k`.
fn draw(title: &str, detail: Option<&str>, w: u32, h: u32, s: f32, k: f32) -> Pixmap {
    let mut pm = Pixmap::new(w, h).unwrap();
    let r = tiny_skia::Rect::from_xywh(0.0, 0.0, w as f32, h as f32).unwrap();
    let bg = paint::rounded_rect(r, RADIUS_PT * k * s);
    pm.fill_path(
        &bg,
        &paint::paint(theme::TOAST_BG, 255),
        FillRule::Winding,
        Transform::identity(),
        None,
    );
    let (title_px, detail_px) = (TITLE_PT * k * s, DETAIL_PT * k * s);
    let x = PAD_PT * k * s;
    let mut y = PAD_PT * 0.8 * k * s;
    let t = text::layout(title, title_px);
    t.draw(
        &mut pm.as_mut(),
        x,
        y,
        title_px,
        &paint::paint(theme::TOAST_TITLE, 255),
        Transform::identity(),
    );
    y += t.h + GAP_PT * k * s;
    if let Some(d) = detail {
        let l = text::layout(d, detail_px);
        l.draw(
            &mut pm.as_mut(),
            x,
            y,
            detail_px,
            &paint::paint(theme::TOAST_DETAIL, 255),
            Transform::identity(),
        );
    }
    pm
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOLD: Duration = Duration::from_millis(1200);

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn long_detail_is_shortened_in_the_middle() {
        let long =
            "Screenshot 2026-11-10 at 09.05.07 with a very very very long name that goes on.png";
        let short = ellipsize(long, DETAIL_PT, 200.0);
        assert!(short.contains('…'));
        assert!(short.ends_with(".png"));
        assert!(text::layout(&short, DETAIL_PT).w <= 200.0);
        assert_eq!(ellipsize("ok", DETAIL_PT, 200.0), "ok");
    }

    #[test]
    fn size_grows_with_detail_and_with_the_size_setting() {
        let (w1, h1) = size_pt("Copied", None, 1.0);
        let (w2, h2) = size_pt("Copied", Some("Click to show in Finder"), 1.0);
        assert!(w2 > w1 && h2 > h1);
        assert!(w2 <= MAX_W_PT);
        let (wl, hl) = size_pt("Copied", None, ToastSize::Large.factor());
        assert!(wl > w1 * 1.5 && hl > h1 * 1.5);
    }

    #[test]
    fn slides_in_stays_then_slides_out() {
        let start = pose(ms(0), HOLD, true).unwrap();
        assert_eq!((start.alpha, start.slide), (0.0, 1.0));
        let mid = pose(ms(100), HOLD, true).unwrap();
        assert!(mid.alpha > 0.0 && mid.alpha < 1.0 && mid.slide > 0.0 && mid.animating);
        let steady = pose(ms(600), HOLD, true).unwrap();
        assert_eq!(
            (steady.alpha, steady.slide, steady.animating),
            (1.0, 0.0, false)
        );
        let leaving = pose(HOLD + ms(100), HOLD, true).unwrap();
        assert!(leaving.alpha < 1.0 && leaving.slide > 0.0 && leaving.animating);
        assert!(pose(HOLD + EXIT, HOLD, true).is_none());
    }

    #[test]
    fn easing_is_monotonic_and_ends_at_one() {
        let samples: Vec<f32> = (0..=10).map(|i| ease_out(i as f32 / 10.0)).collect();
        assert!(samples.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!((ease_out(0.0), ease_out(1.0)), (0.0, 1.0));
        assert_eq!((ease_in(0.0), ease_in(1.0)), (0.0, 1.0));
    }

    #[test]
    fn reduce_motion_just_appears_and_disappears() {
        assert_eq!(
            pose(ms(0), HOLD, false),
            Some(Pose {
                alpha: 1.0,
                slide: 0.0,
                animating: false
            })
        );
        assert!(pose(HOLD, HOLD, false).is_none());
    }

    #[test]
    fn corners_keep_the_offset_from_the_usable_area() {
        // 1500×900 usable area starting below a 30-pt menu bar.
        let area = (0, 30, 1500, 900);
        let size = (300.0, 80.0);
        assert_eq!(place(area, ToastCorner::TopRight, size, 32), (1168, 62));
        assert_eq!(place(area, ToastCorner::TopLeft, size, 32), (32, 62));
        assert_eq!(place(area, ToastCorner::BottomRight, size, 32), (1168, 818));
        assert_eq!(place(area, ToastCorner::BottomLeft, size, 16), (16, 834));
        // A second display to the left of the primary one.
        assert_eq!(
            place((-1000, 0, 1000, 800), ToastCorner::TopRight, size, 20),
            (-320, 20)
        );
    }
}
