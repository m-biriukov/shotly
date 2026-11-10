//! Small `AppKit` helpers that winit does not cover.

use objc2::rc::Retained;
use objc2::{MainThreadMarker, msg_send};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationOptions, NSColor, NSRunningApplication, NSScreen,
    NSScreenSaverWindowLevel, NSStatusWindowLevel, NSView, NSWindow, NSWindowCollectionBehavior,
    NSWorkspace,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

fn ns_window(window: &Window) -> Option<Retained<NSWindow>> {
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::AppKit(h) = handle.as_raw() else {
        return None;
    };
    // SAFETY: winit guarantees `ns_view` is a valid NSView for the window's
    // lifetime, and these helpers run on the main thread inside the event loop.
    let view: &NSView = unsafe { h.ns_view.cast().as_ref() };
    view.window()
}

fn all_spaces() -> NSWindowCollectionBehavior {
    NSWindowCollectionBehavior::CanJoinAllSpaces
        | NSWindowCollectionBehavior::FullScreenAuxiliary
        | NSWindowCollectionBehavior::Stationary
}

/// Puts the overlay above the menu bar and Dock, on every Space, including
/// next to fullscreen apps.
pub fn raise_above_everything(window: &Window) {
    let Some(w) = ns_window(window) else { return };
    w.setLevel(NSScreenSaverWindowLevel);
    w.setCollectionBehavior(all_spaces());
    w.setHasShadow(false);
}

/// Floating, rounded, shadowed window for the toast.
pub fn style_toast(window: &Window, corner_radius: f64) {
    let Some(w) = ns_window(window) else { return };
    w.setLevel(NSStatusWindowLevel);
    w.setCollectionBehavior(all_spaces());
    w.setOpaque(false);
    w.setBackgroundColor(Some(&NSColor::clearColor()));
    w.setHasShadow(true);
    if let Some(view) = w.contentView() {
        view.setWantsLayer(true);
        // SAFETY: plain CALayer property setters on the view's backing layer.
        unsafe {
            let layer: Option<Retained<objc2::runtime::AnyObject>> = msg_send![&*view, layer];
            if let Some(layer) = layer {
                let _: () = msg_send![&*layer, setCornerRadius: corner_radius];
                let _: () = msg_send![&*layer, setMasksToBounds: true];
            }
        }
    }
}

/// Sets a window's opacity (0 = invisible, 1 = opaque).
pub fn set_window_alpha(window: &Window, alpha: f64) {
    if let Some(w) = ns_window(window) {
        w.setAlphaValue(alpha);
    }
}

/// True when the user turned on "Reduce motion" in Accessibility settings.
pub fn reduce_motion() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

/// A display frame in global points: x, y (top-left origin), width, height.
pub type Frame = (i32, i32, u32, u32);

/// Converts a Cocoa rectangle (origin bottom-left of the primary display)
/// into global points with a top-left origin, as `CGDisplayBounds` uses.
fn cocoa_to_global(primary_height: f64, r: objc2_foundation::NSRect) -> Frame {
    (
        r.origin.x.round() as i32,
        (primary_height - (r.origin.y + r.size.height)).round() as i32,
        r.size.width.round() as u32,
        r.size.height.round() as u32,
    )
}

/// The part of the display `display` that windows may use: without the menu
/// bar and the Dock. Falls back to the whole display if it cannot be found.
pub fn usable_area(display: Frame) -> Frame {
    let Some(mtm) = MainThreadMarker::new() else {
        return display;
    };
    let screens = NSScreen::screens(mtm);
    // The first screen is the one with the menu bar; its height anchors the
    // coordinate flip.
    let Some(primary) = screens.firstObject() else {
        return display;
    };
    let ph = primary.frame().size.height;
    for screen in &screens {
        let whole = cocoa_to_global(ph, screen.frame());
        if (whole.0, whole.1) == (display.0, display.1) {
            return cocoa_to_global(ph, screen.visibleFrame());
        }
    }
    display
}

/// Brings this (menu-bar) app to the front so its windows get keyboard input.
pub fn activate_app() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
}

/// The app the user was in, so focus can go back to it after a capture.
pub fn frontmost_other_app() -> Option<Retained<NSRunningApplication>> {
    let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    (app.processIdentifier() != std::process::id() as i32).then_some(app)
}

pub fn reactivate(app: &NSRunningApplication) {
    app.activateWithOptions(NSApplicationActivationOptions::empty());
}

/// Shows `path` selected in a Finder window.
pub fn reveal_in_finder(path: &std::path::Path) {
    if let Err(e) = std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn()
    {
        eprintln!("could not open Finder: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    #[test]
    fn cocoa_rects_flip_to_top_left_origin() {
        // 1000-pt-high primary display: menu bar (30 pt) and Dock (70 pt).
        let visible = NSRect::new(NSPoint::new(0.0, 70.0), NSSize::new(1500.0, 900.0));
        assert_eq!(cocoa_to_global(1000.0, visible), (0, 30, 1500, 900));
        // A second display above and to the right of the primary one.
        let above = NSRect::new(NSPoint::new(1500.0, 1000.0), NSSize::new(800.0, 600.0));
        assert_eq!(cocoa_to_global(1000.0, above), (1500, -600, 800, 600));
    }

    #[test]
    fn usable_area_is_inside_the_display() {
        // Needs the main thread; off it the display is returned unchanged.
        let display = (0, 0, 1512, 982);
        assert_eq!(usable_area(display), display);
    }
}
