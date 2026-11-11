//! Frames of the windows currently on screen, for click-to-select-window.

use std::ffi::c_void;

use objc2_core_foundation::{
    CFArray, CFDictionary, CFNumber, CFNumberType, CFString, CFType, CGRect,
};
use objc2_core_graphics::{
    CGRectMakeWithDictionaryRepresentation, CGWindowListCopyWindowInfo, CGWindowListOption,
};

use crate::geom::{Rect, Scale};

/// A window frame in global points (origin at the primary display's top-left).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// `CGWindowID`; `None` when the frame is not a window.
    pub id: Option<u32>,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Ignore slivers like invisible helper windows.
const MIN_SIDE_PT: f64 = 40.0;

/// Normal app windows (layer 0) on screen, front to back, excluding our own.
pub fn list_windows() -> Vec<Frame> {
    let own_pid = std::process::id() as i32;
    let Some(list) = CGWindowListCopyWindowInfo(
        CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements,
        0,
    ) else {
        return Vec::new();
    };
    let list: &CFArray = &list;
    let mut out = Vec::new();
    for i in 0..list.count() {
        // SAFETY: `i` is below `list.count()`.
        let item = unsafe { list.value_at_index(i) };
        if item.is_null() {
            continue;
        }
        // SAFETY: array elements are CF objects kept alive by `list`; the
        // runtime type is checked before use.
        let Some(dict) = (unsafe { &*(item as *const CFType) }).downcast_ref::<CFDictionary>()
        else {
            continue;
        };
        if number(dict, "kCGWindowLayer") != Some(0) {
            continue;
        }
        if number(dict, "kCGWindowOwnerPID") == Some(own_pid) {
            continue;
        }
        if number(dict, "kCGWindowAlpha").is_some_and(|a| a == 0) {
            continue;
        }
        let Some(frame) = frame_of_dict(dict) else {
            continue;
        };
        if frame.w < MIN_SIDE_PT || frame.h < MIN_SIDE_PT {
            continue;
        }
        out.push(frame);
    }
    out
}

/// A window's number and bounds from one entry of `CGWindowListCopyWindowInfo`.
fn frame_of_dict(dict: &CFDictionary) -> Option<Frame> {
    let r = bounds(dict)?;
    Some(Frame {
        id: number(dict, "kCGWindowNumber")
            .and_then(|n| u32::try_from(n).ok())
            .filter(|&n| n != 0),
        x: r.origin.x,
        y: r.origin.y,
        w: r.size.width,
        h: r.size.height,
    })
}

/// Where window `id` is now, or `None` if it is gone.
pub fn frame_of(id: u32) -> Option<Frame> {
    let list = CGWindowListCopyWindowInfo(CGWindowListOption::OptionIncludingWindow, id)?;
    let list: &CFArray = &list;
    for i in 0..list.count() {
        // SAFETY: `i` is below `list.count()`.
        let item = unsafe { list.value_at_index(i) };
        if item.is_null() {
            continue;
        }
        // SAFETY: array elements are CF objects kept alive by `list`; the
        // runtime type is checked before use.
        let Some(dict) = (unsafe { &*(item as *const CFType) }).downcast_ref::<CFDictionary>()
        else {
            continue;
        };
        if number(dict, "kCGWindowNumber") != Some(id as i32) {
            continue;
        }
        return frame_of_dict(dict);
    }
    None
}

fn value<'a>(dict: &'a CFDictionary, key: &str) -> Option<&'a CFType> {
    let key = CFString::from_str(key);
    // SAFETY: lookup by a CFString key; values are CF objects owned by `dict`.
    let v = unsafe { dict.value((&*key as *const CFString).cast()) };
    if v.is_null() {
        return None;
    }
    // SAFETY: a non-null value is a CF object kept alive by `dict`.
    Some(unsafe { &*v.cast::<CFType>() })
}

/// Integer value of `key`. `None` if missing, not a number, or not exactly
/// representable as an int (so a fractional alpha such as 0.5 is `None`).
fn number(dict: &CFDictionary, key: &str) -> Option<i32> {
    let n = value(dict, key)?.downcast_ref::<CFNumber>()?;
    let mut out: i32 = 0;
    // SAFETY: `n` is a CFNumber (checked above) and `out` is a valid C int.
    let ok = unsafe { n.value(CFNumberType::IntType, &mut out as *mut i32 as *mut c_void) };
    ok.then_some(out)
}

fn bounds(dict: &CFDictionary) -> Option<CGRect> {
    let b = value(dict, "kCGWindowBounds")?.downcast_ref::<CFDictionary>()?;
    let mut r = CGRect::default();
    // SAFETY: `b` is a CFDictionary (checked above); `r` is a valid out-pointer.
    let ok = unsafe { CGRectMakeWithDictionaryRepresentation(Some(b), &mut r) };
    ok.then_some(r)
}

/// Converts global frames into the pixel space of one display's overlay,
/// clipped to it, keeping front-to-back order and dropping ones not on it.
pub fn to_display(
    frames: &[Frame],
    origin_pt: (i32, i32),
    scale: Scale,
    bounds: Rect,
) -> Vec<Rect> {
    let scale = scale.get();
    frames
        .iter()
        .map(|f| {
            let x = ((f.x - origin_pt.0 as f64) * scale).round() as i32;
            let y = ((f.y - origin_pt.1 as f64) * scale).round() as i32;
            Rect::new(
                x,
                y,
                (f.w * scale).round() as i32,
                (f.h * scale).round() as i32,
            )
            .intersect(&bounds)
        })
        .filter(|r| !r.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_and_clips_to_display() {
        let frames = [
            Frame {
                id: Some(1),
                x: 100.0,
                y: 50.0,
                w: 200.0,
                h: 100.0,
            },
            Frame {
                id: Some(2),
                x: -500.0,
                y: 0.0,
                w: 400.0,
                h: 300.0,
            },
            Frame {
                id: Some(3),
                x: 1400.0,
                y: 900.0,
                w: 400.0,
                h: 300.0,
            },
        ];
        let bounds = Rect::new(0, 0, 3024, 1964);
        let r = to_display(&frames, (0, 0), Scale(2.0), bounds);
        assert_eq!(
            r,
            vec![
                Rect::new(200, 100, 400, 200),
                Rect::new(2800, 1800, 224, 164)
            ]
        );
        // Second display to the left at x = -1000 pt, 1x.
        let r = to_display(&frames, (-1000, 0), Scale(1.0), Rect::new(0, 0, 1000, 800));
        assert_eq!(r, vec![Rect::new(500, 0, 400, 300)]);
    }

    #[test]
    #[ignore = "needs windows on screen"]
    fn a_listed_window_is_found_again_by_its_id() {
        let listed = list_windows();
        let Some(first) = listed.iter().find(|f| f.id.is_some()) else {
            return;
        };
        let again = frame_of(first.id.unwrap()).expect("window is still there");
        assert_eq!(again.id, first.id);
        assert!((again.w - first.w).abs() < 1.0 && (again.h - first.h).abs() < 1.0);
        assert_eq!(frame_of(u32::MAX), None);
    }

    #[test]
    #[ignore]
    fn print_live_windows() {
        for f in list_windows() {
            println!("{f:?}");
        }
    }
}
