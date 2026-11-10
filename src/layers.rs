//! Core Animation layer tree for one overlay window.
//!
//! The frozen screen is a GPU layer set once; dimming is four translucent
//! layers around the selection; borders and handles are plain layers. Only
//! shapes are rasterised on the CPU: the static ones when they change, and
//! the one being drawn within its own small bounding box. Moving the
//! selection or hovering therefore costs a few frame changes, not a repaint.

use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_core_foundation::{CFData, CFRetained, CGFloat, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapInfo, CGColor, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage,
    CGImageAlphaInfo,
};
use objc2_foundation::{NSNumber, ns_string};
use objc2_quartz_core::{
    CABasicAnimation, CALayer, CAMediaTiming, CAMediaTimingFunction, CATransaction,
    kCAMediaTimingFunctionEaseOut,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tiny_skia::Pixmap;
use winit::window::Window;

use crate::editor::Editor;
use crate::geom::{Handle, Rect, Scale};
use crate::paint;
use crate::render::{self, Backdrop};
use crate::shape::ShapeHandle;
use crate::theme::{self, ACCENT};
use crate::toolbar;

pub struct Layers {
    scale: Scale,
    bounds: Rect,
    space: CFRetained<CGColorSpace>,
    _root: Retained<CALayer>,
    /// Holds the four dimming layers so they fade in as one.
    dim_group: Retained<CALayer>,
    dim: [Retained<CALayer>; 4],
    clip: Retained<CALayer>,
    ink: Retained<CALayer>,
    active: Retained<CALayer>,
    border: Retained<CALayer>,
    area_handles: Vec<Retained<CALayer>>,
    sel_outline: Retained<CALayer>,
    sel_handles: Vec<Retained<CALayer>>,
    caret: Retained<CALayer>,
    toolbar: Retained<CALayer>,
    flyout: Retained<CALayer>,
    label: Retained<CALayer>,
    tip: Retained<CALayer>,
    /// Caches so unchanged rasters are not redone.
    ink_key: Option<u64>,
    ink_rect: Rect,
    toolbar_key: Option<(Rect, toolbar::State)>,
    flyout_key: Option<(Rect, toolbar::State)>,
    label_key: Option<(i32, i32)>,
    tip_key: Option<String>,
    tip_size: (i32, i32),
}

fn color(rgb: u32, alpha: f64) -> CFRetained<CGColor> {
    let c = |shift: u32| ((rgb >> shift) & 0xFF) as CGFloat / 255.0;
    CGColor::new_srgb(c(16), c(8), c(0), alpha)
}

fn new_layer(parent: &CALayer) -> Retained<CALayer> {
    let l = CALayer::new();
    l.setHidden(true);
    parent.addSublayer(&l);
    l
}

impl Layers {
    /// Builds the layer tree inside `window`'s view, showing `screen`.
    pub fn attach(window: &Window, screen: &CGImage, scale: Scale, bounds: Rect) -> Option<Self> {
        let handle = window.window_handle().ok()?;
        let RawWindowHandle::AppKit(h) = handle.as_raw() else {
            return None;
        };
        // SAFETY: winit guarantees `ns_view` is a valid NSView on the main thread.
        let view: &objc2_app_kit::NSView = unsafe { h.ns_view.cast().as_ref() };
        view.setWantsLayer(true);
        // SAFETY: plain property read of a layer-backed view.
        let host: Option<Retained<CALayer>> = unsafe { msg_send![view, layer] };
        let host = host?;
        let space = CGImage::color_space(Some(screen)).or_else(CGColorSpace::new_device_rgb)?;

        CATransaction::begin();
        CATransaction::setDisableActions(true);
        let root = CALayer::new();
        // Top-left origin like everything else in this app. winit's view is
        // usually flipped already; flipping again would turn it back.
        root.setGeometryFlipped(!host.isGeometryFlipped());
        root.setFrame(rect_pt(bounds, scale));
        host.addSublayer(&root);

        let bg = CALayer::new();
        bg.setFrame(rect_pt(bounds, scale));
        bg.setContentsScale(scale.get());
        set_contents(&bg, Some(screen));
        root.addSublayer(&bg);

        let dim_group = CALayer::new();
        dim_group.setFrame(rect_pt(bounds, scale));
        root.addSublayer(&dim_group);
        let dim = std::array::from_fn(|_| {
            let l = new_layer(&dim_group);
            l.setBackgroundColor(Some(&color(0, 0.5)));
            l
        });
        let clip = new_layer(&root);
        clip.setMasksToBounds(true);
        let ink = new_layer(&clip);
        let active = new_layer(&clip);
        for l in [&ink, &active] {
            l.setContentsScale(scale.get());
        }
        let border = new_layer(&root);
        border.setBorderColor(Some(&color(ACCENT, 1.0)));
        border.setBorderWidth(scale.pt(scale.hairline()));
        let handle_layer = |root: &CALayer| {
            let l = new_layer(root);
            l.setBackgroundColor(Some(&color(theme::HANDLE_FILL, 1.0)));
            l.setBorderColor(Some(&color(ACCENT, 1.0)));
            l.setBorderWidth(scale.pt(scale.hairline()));
            l
        };
        let area_handles = (0..8).map(|_| handle_layer(&root)).collect();
        let sel_outline = new_layer(&root);
        sel_outline.setBorderColor(Some(&color(ACCENT, 1.0)));
        sel_outline.setBorderWidth(scale.pt(1));
        let sel_handles = (0..8).map(|_| handle_layer(&root)).collect();
        let caret = new_layer(&root);
        let toolbar = new_layer(&root);
        let flyout = new_layer(&root);
        let label = new_layer(&root);
        // Last, so it draws above everything else.
        let tip = new_layer(&root);
        for l in [&toolbar, &flyout, &label, &tip] {
            l.setContentsScale(scale.get());
        }
        CATransaction::commit();

        Some(Self {
            scale,
            bounds,
            space,
            _root: root,
            dim_group,
            dim,
            clip,
            ink,
            active,
            border,
            area_handles,
            sel_outline,
            sel_handles,
            caret,
            toolbar,
            flyout,
            label,
            tip,
            ink_key: None,
            ink_rect: Rect::new(0, 0, 0, 0),
            toolbar_key: None,
            flyout_key: None,
            label_key: None,
            tip_key: None,
            tip_size: (0, 0),
        })
    }

    /// Fades the dimming in, so the screen eases into the capture state
    /// instead of snapping. Call right after the window is shown.
    pub fn animate_start(&self) {
        fade_in(&self.dim_group, DIM_FADE_SECS);
    }

    fn pt(&self, r: Rect) -> CGRect {
        rect_pt(r, self.scale)
    }

    /// Brings every layer in line with the editor. `bd` is only needed (and
    /// only consulted) when there are shapes to paint.
    pub fn sync(&mut self, ed: &Editor, bd: Option<&Backdrop>, tip_visible: bool) {
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        let preview = ed.preview();
        let area = ed.area();
        let shown = area.or(preview);
        let t = self.scale.hairline();

        // Dim everything except the selection (or the hovered window).
        let b = self.bounds;
        let rects = match shown {
            None => [
                b,
                Rect::new(0, 0, 0, 0),
                Rect::new(0, 0, 0, 0),
                Rect::new(0, 0, 0, 0),
            ],
            Some(a) => [
                Rect::new(b.x, b.y, b.w, a.y - b.y),
                Rect::new(b.x, a.bottom(), b.w, b.bottom() - a.bottom()),
                Rect::new(b.x, a.y, a.x - b.x, a.h),
                Rect::new(a.right(), a.y, b.right() - a.right(), a.h),
            ],
        };
        for (l, r) in self.dim.iter().zip(rects) {
            show(l, !r.is_empty());
            l.setFrame(self.pt(r));
        }

        match shown {
            Some(a) => {
                show(&self.border, true);
                self.border.setFrame(self.pt(a.expand(t)));
            }
            None => show(&self.border, false),
        }

        // Selection handles.
        let half = self.scale.px(4.0);
        let area_handles = area.filter(|_| !ed.dragging_area());
        for (l, h) in self.area_handles.iter().zip(Handle::ALL) {
            show(l, area_handles.is_some());
            if let Some(a) = area_handles {
                let c = h.anchor(&a);
                l.setFrame(self.pt(Rect::around(c, half)));
            }
        }

        // Shapes, clipped to the selection.
        show(&self.clip, area.is_some());
        if let Some(a) = area {
            self.clip.setFrame(self.pt(a));
            self.sync_ink(ed, bd, a);
            self.sync_active(ed, bd, a);
        }

        self.sync_selected(ed);
        self.sync_caret(ed);
        self.sync_toolbar(ed);
        self.sync_label(ed, shown);
        self.sync_tip(ed, tip_visible);
        CATransaction::commit();
    }

    fn sync_ink(&mut self, ed: &Editor, bd: Option<&Backdrop>, area: Rect) {
        let key = ed.static_key();
        if self.ink_key != Some(key) {
            self.ink_key = Some(key);
            let bounds = ed
                .static_shapes()
                .map(render::paint_bounds)
                .reduce(|a, b| a.union(&b));
            let painted = bounds
                .zip(bd)
                .and_then(|(r, bd)| render::region(bd, r, ed.static_shapes()));
            match painted {
                Some((r, pm)) => {
                    self.ink_rect = r;
                    set_contents(&self.ink, Some(&self.image(&pm)));
                    show(&self.ink, true);
                }
                None => {
                    set_contents(&self.ink, None);
                    show(&self.ink, false);
                }
            }
        }
        // Shapes stay put on screen while the selection moves.
        let r = self.ink_rect;
        self.ink
            .setFrame(self.pt(Rect::new(r.x - area.x, r.y - area.y, r.w, r.h)));
    }

    fn sync_active(&mut self, ed: &Editor, bd: Option<&Backdrop>, area: Rect) {
        let painted = ed
            .active_shape()
            .zip(bd)
            .and_then(|(s, bd)| render::active(bd, s));
        let Some((r, pm, multiply)) = painted else {
            set_contents(&self.active, None);
            show(&self.active, false);
            return;
        };
        set_contents(&self.active, Some(&self.image(&pm)));
        let filter = multiply.then(|| ns_string!("multiplyBlendMode"));
        // SAFETY: compositingFilter accepts a filter name string.
        unsafe {
            self.active
                .setCompositingFilter(filter.map(std::convert::AsRef::as_ref));
        };
        self.active
            .setFrame(self.pt(Rect::new(r.x - area.x, r.y - area.y, r.w, r.h)));
        show(&self.active, true);
    }

    fn sync_selected(&mut self, ed: &Editor) {
        let sel = ed.selected_shape().or(ed.editing_shape());
        let handles = sel.map(super::shape::Shape::handles).unwrap_or_default();
        let outline =
            sel.filter(|_| handles.is_empty() || matches!(handles[0].0, ShapeHandle::Box(_)));
        show(&self.sel_outline, outline.is_some());
        if let Some(s) = outline {
            self.sel_outline.setFrame(self.pt(s.bbox()));
        }
        let half = self.scale.px(3.5);
        let editing = ed.editing_shape().is_some();
        for (i, l) in self.sel_handles.iter().enumerate() {
            let c = handles.get(i).map(|(_, c)| *c).filter(|_| !editing);
            show(l, c.is_some());
            if let Some(c) = c {
                l.setFrame(self.pt(Rect::around(c, half)));
            }
        }
    }

    fn sync_caret(&mut self, ed: &Editor) {
        let (Some((x, y, h)), Some(s)) = (ed.caret(), ed.editing_shape()) else {
            return show(&self.caret, false);
        };
        let c = if s.style.bg {
            paint::contrast(s.style.color)
        } else {
            s.style.color
        };
        self.caret.setBackgroundColor(Some(&color(c, 1.0)));
        let w = self.scale.pxf(1.5);
        let s = self.scale.get();
        self.caret.setFrame(CGRect::new(
            CGPoint::new(((x - w / 2.0) as f64) / s, y as f64 / s),
            CGSize::new(w as f64 / s, h as f64 / s),
        ));
        show(&self.caret, true);
    }

    fn sync_toolbar(&mut self, ed: &Editor) {
        let state = ed.toolbar_state();
        let (layer, key) = (&self.toolbar, &mut self.toolbar_key);
        sync_bar(layer, key, ed.toolbar(), &state, self.scale, &self.space);
        let (layer, key) = (&self.flyout, &mut self.flyout_key);
        sync_bar(layer, key, ed.flyout(), &state, self.scale, &self.space);
    }

    /// Tooltip for the hovered toolbar button, once the hover delay passed.
    fn sync_tip(&mut self, ed: &Editor, visible: bool) {
        let Some(t) = ed.hover_target().filter(|_| visible) else {
            return show(&self.tip, false);
        };
        let appearing = self.tip.isHidden();
        let text = toolbar::tooltip(t.item);
        if self.tip_key.as_deref() != Some(text.as_str()) {
            let pm = render::pill(&text, self.scale);
            set_contents(&self.tip, Some(&self.image(&pm)));
            self.tip_size = (pm.width() as i32, pm.height() as i32);
            self.tip_key = Some(text);
        }
        let gap = self.scale.px(6.0);
        let r = toolbar::place_tooltip(self.tip_size, t.rect, t.bar, t.other, self.bounds, gap);
        self.tip.setFrame(self.pt(r));
        show(&self.tip, true);
        if appearing {
            fade_in(&self.tip, POPUP_FADE_SECS);
        }
    }

    fn sync_label(&mut self, ed: &Editor, shown: Option<Rect>) {
        let Some(a) = shown else {
            return show(&self.label, false);
        };
        if self.label_key != Some((a.w, a.h)) {
            let pm = render::size_label(a.w, a.h, self.scale);
            set_contents(&self.label, Some(&self.image(&pm)));
            self.label.setBounds(CGRect::new(
                CGPoint::new(0.0, 0.0),
                CGSize::new(
                    self.scale.pt(pm.width() as i32),
                    self.scale.pt(pm.height() as i32),
                ),
            ));
            self.label_key = Some((a.w, a.h));
        }
        // Above the top-left corner, or just inside when there is no room or
        // the toolbar sits above the selection.
        let lh = self.scale.px(self.label.bounds().size.height);
        let gap = self.scale.px(6.0);
        let toolbar_above = ed.toolbar().is_some_and(|t| t.rect.bottom() <= a.y);
        let mut y = a.y - gap - lh;
        if y < 0 || toolbar_above {
            y = a.y + gap;
        }
        let size = self.label.bounds().size;
        self.label.setFrame(CGRect::new(
            CGPoint::new(self.scale.pt(a.x), self.scale.pt(y)),
            size,
        ));
        show(&self.label, true);
    }

    fn image(&self, pm: &Pixmap) -> CFRetained<CGImage> {
        image(pm, &self.space)
    }
}

/// Shows `bar` (toolbar or flyout) in `layer`, redrawing only when its
/// position or highlighted state changed.
fn sync_bar(
    layer: &CALayer,
    key: &mut Option<(Rect, toolbar::State)>,
    bar: Option<toolbar::Toolbar>,
    state: &toolbar::State,
    scale: Scale,
    space: &CGColorSpace,
) {
    let Some(bar) = bar else {
        *key = None;
        return show(layer, false);
    };
    let appearing = layer.isHidden();
    let k = (bar.rect, state.clone());
    if key.as_ref() != Some(&k) {
        let pm = bar.draw(state, scale);
        set_contents(layer, Some(&image(&pm, space)));
        layer.setFrame(rect_pt(bar.rect, scale));
        *key = Some(k);
    }
    show(layer, true);
    if appearing {
        fade_in(layer, POPUP_FADE_SECS);
    }
}

/// Dimming fade at the start of a capture, and toolbar/flyout/tooltip fades.
const DIM_FADE_SECS: f64 = 0.22;
const POPUP_FADE_SECS: f64 = 0.14;

/// Animates `layer`'s opacity from transparent to opaque. Does nothing when
/// the user turned on "Reduce motion" in Accessibility settings.
fn fade_in(layer: &CALayer, seconds: f64) {
    if crate::macos::reduce_motion() {
        return;
    }
    let anim = CABasicAnimation::animationWithKeyPath(Some(ns_string!("opacity")));
    let (from, to) = (
        NSNumber::numberWithFloat(0.0),
        NSNumber::numberWithFloat(1.0),
    );
    // SAFETY: from/to are NSNumbers, the value type CABasicAnimation expects
    // for "opacity"; the timing-function name is a constant from QuartzCore.
    unsafe {
        anim.setFromValue(Some(from.as_ref()));
        anim.setToValue(Some(to.as_ref()));
        anim.setTimingFunction(Some(&CAMediaTimingFunction::functionWithName(
            kCAMediaTimingFunctionEaseOut,
        )));
    }
    anim.setDuration(seconds);
    // The layer's own opacity stays 1, so nothing snaps when the animation ends.
    layer.addAnimation_forKey(&anim, Some(ns_string!("fade-in")));
}

/// Wraps premultiplied RGBA pixels as a `CGImage` in the screen's colour
/// space, so painted regions match the screen layer exactly.
fn image(pm: &Pixmap, space: &CGColorSpace) -> CFRetained<CGImage> {
    let data = CFData::from_bytes(pm.data());
    let provider = CGDataProvider::with_cf_data(Some(&data)).expect("data provider");
    let w = pm.width() as usize;
    // SAFETY: the provider holds w*h*4 bytes of RGBA8 premultiplied data.
    unsafe {
        CGImage::new(
            w,
            pm.height() as usize,
            8,
            32,
            w * 4,
            Some(space),
            CGBitmapInfo(CGImageAlphaInfo::PremultipliedLast.0),
            Some(&provider),
            std::ptr::null(),
            false,
            CGColorRenderingIntent::RenderingIntentDefault,
        )
    }
    .expect("CGImage")
}

fn show(l: &CALayer, on: bool) {
    l.setHidden(!on);
}

fn rect_pt(r: Rect, scale: Scale) -> CGRect {
    CGRect::new(
        CGPoint::new(scale.pt(r.x), scale.pt(r.y)),
        CGSize::new(scale.pt(r.w), scale.pt(r.h)),
    )
}

fn set_contents(l: &CALayer, img: Option<&CGImage>) {
    // SAFETY: CGImage is a CF type toll-free usable as layer contents.
    unsafe {
        let obj = img.map(|i| &*(i as *const CGImage as *const AnyObject));
        l.setContents(obj);
    }
}
