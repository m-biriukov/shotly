//! What the viewer of a recording sees besides the screen: a halo around the
//! pointer, a ripple at every mouse click, a frame around the recorded area
//! and the 3-2-1 countdown. All of them are small transparent click-through
//! windows; the halo and the ripples are *meant* to be captured (the frame
//! sits just outside the recorded area, the countdown ends before recording
//! starts).
//!
//! The pointer position and pressed buttons are polled every frame
//! (`NSEvent.mouseLocation` / `pressedMouseButtons`), which needs no
//! Accessibility or Input Monitoring permission.

use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::Result;
use tiny_skia::{FillRule, PathBuilder, Pixmap, Transform};
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

use crate::config::Config;
use crate::macos;
use crate::paint::{paint, round_stroke};
use crate::snap::Frame;
use crate::style::PALETTE;
use crate::text;

/// Pointer polling and ripple animation interval (~60 fps).
const FRAME: Duration = Duration::from_millis(16);
/// How long a click ripple lasts.
const RIPPLE: Duration = Duration::from_millis(520);
/// Ripple window size in points, and how many may be on screen at once.
const RIPPLE_PT: f64 = 110.0;
const MAX_RIPPLES: usize = 6;
/// Thickness of the frame around the recorded area, in points.
const OUTLINE_PT: f64 = 3.0;
const OUTLINE_COLOR: u32 = 0xFF3B30;
/// Title of every window in this module. A recording excludes the app's other
/// windows (Settings, notices) but keeps these.
pub const EFFECT_TITLE: &str = "shotly-effect";

fn window(
    el: &ActiveEventLoop,
    at: (f64, f64),
    size: (f64, f64),
    fill: Option<u32>,
) -> Result<Rc<Window>> {
    let attrs = Window::default_attributes()
        .with_title(EFFECT_TITLE)
        .with_decorations(false)
        .with_resizable(false)
        .with_active(false)
        .with_visible(false)
        .with_transparent(true)
        .with_position(LogicalPosition::new(at.0, at.1))
        .with_inner_size(LogicalSize::new(size.0, size.1));
    let window = Rc::new(el.create_window(attrs)?);
    macos::style_effect(&window, fill);
    // Without this the window would swallow the clicks meant for what is
    // under it, so a failure here must not leave it on screen.
    window
        .set_cursor_hittest(false)
        .map_err(|e| anyhow::anyhow!("could not make an overlay click-through: {e}"))?;
    Ok(window)
}

/// A square window that is centred on a point and shows one picture.
struct Spot {
    window: Rc<Window>,
    size: f64,
    shown: bool,
}

impl Spot {
    fn new(el: &ActiveEventLoop, size: f64) -> Result<Self> {
        Ok(Self {
            window: window(el, (0.0, 0.0), (size, size), None)?,
            size,
            shown: false,
        })
    }

    fn centre_on(&self, p: (f64, f64)) {
        self.window.set_outer_position(LogicalPosition::new(
            (p.0 - self.size / 2.0).round(),
            (p.1 - self.size / 2.0).round(),
        ));
    }

    fn draw(&mut self, pm: &Pixmap) {
        let scale = self.window.scale_factor();
        macos::set_window_image(&self.window, pm, scale);
        if !self.shown {
            self.shown = true;
            self.window.set_visible(true);
        }
    }

    fn hide(&mut self) {
        if std::mem::replace(&mut self.shown, false) {
            self.window.set_visible(false);
        }
    }

    fn pixmap(&self) -> (Pixmap, f32) {
        let s = self.window.scale_factor() as f32;
        let px = (self.size as f32 * s).round().max(1.0) as u32;
        (Pixmap::new(px, px).expect("non-empty pixmap"), s)
    }
}

/// Halo: a soft disc with a ring, `radius_pt` big, in palette colour `rgb`.
fn halo_picture(spot: &Spot, radius_pt: f32, rgb: u32) -> Pixmap {
    let (mut pm, s) = spot.pixmap();
    let c = pm.width() as f32 / 2.0;
    if let Some(disc) = PathBuilder::from_circle(c, c, radius_pt * s) {
        pm.fill_path(
            &disc,
            &paint(rgb, 80),
            FillRule::Winding,
            Transform::identity(),
            None,
        );
        pm.stroke_path(
            &disc,
            &paint(rgb, 215),
            &round_stroke(2.5 * s),
            Transform::identity(),
            None,
        );
    }
    pm
}

/// Ripple `t` (0..1) of the way through its life: a ring that grows and fades.
fn ripple_picture(spot: &Spot, t: f32, rgb: u32) -> Pixmap {
    let (mut pm, s) = spot.pixmap();
    let c = pm.width() as f32 / 2.0;
    let eased = 1.0 - (1.0 - t).powi(3);
    let radius = (10.0 + 36.0 * eased) * s;
    let alpha = (235.0 * (1.0 - t)).round() as u8;
    if let Some(ring) = PathBuilder::from_circle(c, c, radius) {
        pm.stroke_path(
            &ring,
            &paint(rgb, alpha),
            &round_stroke((5.0 - 2.5 * t) * s),
            Transform::identity(),
            None,
        );
    }
    pm
}

struct Ripple {
    spot: Spot,
    /// `None` while the window is free to be reused.
    started: Option<(Instant, u32)>,
}

/// The pointer halo and click ripples during a recording.
pub struct Effects {
    halo: Option<Spot>,
    halo_at: Option<(i64, i64)>,
    ripples: Vec<Ripple>,
    clicks: Option<[u32; 2]>,
    pressed: usize,
}

impl Effects {
    pub fn new(el: &ActiveEventLoop, cfg: &Config) -> Self {
        let halo = cfg
            .highlight_cursor
            .then(|| {
                let r = cfg.highlight_size.radius_pt();
                match Spot::new(el, f64::from(2.0 * r + 12.0)) {
                    Ok(mut spot) => {
                        let pic = halo_picture(&spot, r, PALETTE[cfg.highlight_color]);
                        spot.draw(&pic);
                        Some(spot)
                    }
                    Err(e) => {
                        eprintln!("pointer halo failed: {e:#}");
                        None
                    }
                }
            })
            .flatten();
        Self {
            halo,
            halo_at: None,
            ripples: Vec::new(),
            clicks: cfg.show_clicks.then(|| {
                [
                    PALETTE[cfg.left_click_color],
                    PALETTE[cfg.right_click_color],
                ]
            }),
            pressed: macos::pressed_mouse_buttons(),
        }
    }

    /// Follows the pointer and animates ripples. Returns when to call again.
    pub fn tick(&mut self, el: &ActiveEventLoop, now: Instant) -> Option<Instant> {
        if self.halo.is_none() && self.clicks.is_none() {
            return None;
        }
        let mouse = macos::mouse_location();
        if let (Some(halo), Some(m)) = (&self.halo, mouse) {
            let key = (m.0.round() as i64, m.1.round() as i64);
            if self.halo_at != Some(key) {
                self.halo_at = Some(key);
                halo.centre_on(m);
            }
        }
        let buttons = macos::pressed_mouse_buttons();
        let down = buttons & !self.pressed;
        self.pressed = buttons;
        if let (Some(colors), Some(m)) = (self.clicks, mouse) {
            for (bit, rgb) in [(0b01, colors[0]), (0b10, colors[1])] {
                if down & bit != 0 {
                    self.start_ripple(el, m, rgb, now);
                }
            }
        }
        for r in &mut self.ripples {
            let Some((since, rgb)) = r.started else {
                continue;
            };
            match ripple_progress(now.saturating_duration_since(since)) {
                Some(t) => {
                    let pic = ripple_picture(&r.spot, t, rgb);
                    r.spot.draw(&pic);
                }
                None => {
                    r.started = None;
                    r.spot.hide();
                }
            }
        }
        Some(now + FRAME)
    }

    fn start_ripple(&mut self, el: &ActiveEventLoop, at: (f64, f64), rgb: u32, now: Instant) {
        let free = self.ripples.iter().position(|r| r.started.is_none());
        let i = match free {
            Some(i) => i,
            None if self.ripples.len() < MAX_RIPPLES => match Spot::new(el, RIPPLE_PT) {
                Ok(spot) => {
                    self.ripples.push(Ripple {
                        spot,
                        started: None,
                    });
                    self.ripples.len() - 1
                }
                Err(e) => return eprintln!("ripple window failed: {e:#}"),
            },
            None => return,
        };
        let r = &mut self.ripples[i];
        r.spot.centre_on(at);
        r.started = Some((now, rgb));
    }
}

/// The digit to show `elapsed` into a countdown of `seconds`, and how long
/// after its start that digit changes. `None` once the countdown is over.
fn countdown_step(elapsed: Duration, seconds: u32) -> Option<(u32, Duration)> {
    let passed = u32::try_from(elapsed.as_secs()).unwrap_or(u32::MAX);
    let left = seconds.checked_sub(passed).filter(|&n| n > 0)?;
    Some((left, Duration::from_secs(u64::from(seconds - left) + 1)))
}

/// How far (0..1) a click ripple has come `elapsed` after the click; `None`
/// once it is over.
fn ripple_progress(elapsed: Duration) -> Option<f32> {
    let t = elapsed.as_secs_f32() / RIPPLE.as_secs_f32();
    (t < 1.0).then_some(t)
}

/// A frame just outside the recorded area, so it is visible on screen but
/// not part of the video.
pub struct Outline {
    bars: Vec<Rc<Window>>,
    area: Option<Frame>,
}

impl Outline {
    pub fn new(el: &ActiveEventLoop, area: Frame) -> Result<Self> {
        let mut bars = Vec::with_capacity(4);
        for r in bar_rects(area) {
            bars.push(window(el, (r.0, r.1), (r.2, r.3), Some(OUTLINE_COLOR))?);
        }
        let mut o = Self { bars, area: None };
        o.set_area(area);
        for b in &o.bars {
            b.set_visible(true);
        }
        Ok(o)
    }

    pub fn set_area(&mut self, area: Frame) {
        if self.area == Some(area) {
            return;
        }
        self.area = Some(area);
        for (b, r) in self.bars.iter().zip(bar_rects(area)) {
            b.set_outer_position(LogicalPosition::new(r.0, r.1));
            let _ = b.request_inner_size(LogicalSize::new(r.2, r.3));
        }
    }
}

/// Top, bottom, left and right bars as (x, y, w, h), all outside `area`.
fn bar_rects(a: Frame) -> [(f64, f64, f64, f64); 4] {
    let t = OUTLINE_PT;
    [
        (a.x - t, a.y - t, a.w + 2.0 * t, t),
        (a.x - t, a.y + a.h, a.w + 2.0 * t, t),
        (a.x - t, a.y, t, a.h),
        (a.x + a.w, a.y, t, a.h),
    ]
}

/// The 3-2-1 countdown in the middle of the area about to be recorded.
pub struct Countdown {
    spot: Spot,
    started: Instant,
    shown: Option<u32>,
    seconds: u32,
}

impl Countdown {
    pub fn new(
        el: &ActiveEventLoop,
        centre: (f64, f64),
        seconds: u32,
        now: Instant,
    ) -> Result<Self> {
        let spot = Spot::new(el, 140.0)?;
        spot.centre_on(centre);
        let mut c = Self {
            spot,
            started: now,
            shown: None,
            seconds,
        };
        c.update(now);
        Ok(c)
    }

    /// Takes the countdown off screen now (dropping it closes the window a
    /// little later).
    pub fn hide(&mut self) {
        self.spot.hide();
    }

    /// Shows the right digit for `now`. `None` once the countdown is over,
    /// otherwise when to look again.
    pub fn update(&mut self, now: Instant) -> Option<Instant> {
        let elapsed = now.saturating_duration_since(self.started);
        let (digit, changes_at) = countdown_step(elapsed, self.seconds)?;
        if self.shown != Some(digit) {
            self.shown = Some(digit);
            let pic = digit_picture(&self.spot, digit);
            self.spot.draw(&pic);
        }
        Some(self.started + changes_at)
    }
}

fn digit_picture(spot: &Spot, n: u32) -> Pixmap {
    let (mut pm, s) = spot.pixmap();
    let c = pm.width() as f32 / 2.0;
    if let Some(disc) = PathBuilder::from_circle(c, c, c - 2.0 * s) {
        pm.fill_path(
            &disc,
            &paint(0x1E1E20, 215),
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
    let size = 72.0 * s;
    let label = n.to_string();
    let l = text::layout(&label, size);
    l.draw(
        &mut pm.as_mut(),
        c - l.w / 2.0,
        c - l.h / 2.0,
        size,
        &paint(0xFFFFFF, 255),
        Transform::identity(),
    );
    pm
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn countdown_counts_down_and_says_when_to_look_again() {
        let ms = Duration::from_millis;
        assert_eq!(countdown_step(ms(0), 3), Some((3, ms(1000))));
        assert_eq!(countdown_step(ms(999), 3), Some((3, ms(1000))));
        assert_eq!(countdown_step(ms(1000), 3), Some((2, ms(2000))));
        assert_eq!(countdown_step(ms(2900), 3), Some((1, ms(3000))));
        assert_eq!(countdown_step(ms(3000), 3), None);
        assert_eq!(countdown_step(ms(1), 0), None);
        assert_eq!(countdown_step(Duration::from_secs(u64::MAX / 2), 3), None);
    }

    #[test]
    fn ripples_grow_and_end() {
        assert_eq!(ripple_progress(Duration::ZERO), Some(0.0));
        let mid = ripple_progress(RIPPLE / 2).unwrap();
        assert!((mid - 0.5).abs() < 0.01, "{mid}");
        assert_eq!(ripple_progress(RIPPLE), None);
        assert_eq!(ripple_progress(RIPPLE * 3), None);
    }

    #[test]
    fn frame_bars_surround_the_area_without_overlapping_it() {
        let a = Frame {
            id: None,
            x: 100.0,
            y: 50.0,
            w: 400.0,
            h: 300.0,
        };
        for (x, y, w, h) in bar_rects(a) {
            let overlaps = x < a.x + a.w && x + w > a.x && y < a.y + a.h && y + h > a.y;
            assert!(!overlaps, "bar ({x}, {y}, {w}, {h}) touches the recording");
        }
    }
}
