//! Grabs a frozen image of every display before the overlay appears.
//!
//! Capture goes through `ScreenCaptureKit` (`SCScreenshotManager`, macOS 14+).
//! Its API is asynchronous; each call is bridged to a plain blocking call
//! with a channel, and the displays are captured concurrently. The list of
//! displays and our own running-application entry (`SCShareableContent`) is
//! slow to build, so it is fetched once in [`warm_up`] and reused.
//!
//! The `CGImage` is shown as-is (a Core Animation layer uploads it to the GPU
//! once); the RGBA copy used for drawing and export is converted on a
//! background thread so the overlay does not wait for it.

use std::cell::{OnceCell, RefCell};
use std::ptr::NonNull;
use std::sync::{Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{Context, Result};
use block2::RcBlock;
use image::RgbaImage;
use objc2::rc::Retained;
use objc2::{AnyThread, Message};
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::{
    CGDataProvider, CGDirectDisplayID, CGDisplayBounds, CGDisplayCopyDisplayMode, CGDisplayMode,
    CGGetActiveDisplayList, CGImage, CGImageAlphaInfo, CGImageByteOrderInfo, CGMainDisplayID,
    CGPreflightScreenCaptureAccess, CGRequestScreenCaptureAccess,
};
use objc2_foundation::{NSArray, NSError};
use objc2_screen_capture_kit::{
    SCContentFilter, SCDisplay, SCRunningApplication, SCScreenshotManager, SCShareableContent,
    SCStreamConfiguration,
};

/// `ScreenCaptureKit` never calls back if something goes wrong in the daemon;
/// better an error than a frozen menu-bar app.
const SCK_TIMEOUT: Duration = Duration::from_secs(5);

/// One display's frozen contents plus where it sits in global coordinates.
pub struct Shot {
    /// Top-left corner in global points (CG coordinates: origin at the
    /// primary display's top-left, y grows downwards).
    pub origin_pt: (i32, i32),
    /// Display size in points.
    pub size_pt: (u32, u32),
    /// The captured image, in the display's colour space.
    pub cg: CFRetained<CGImage>,
    pixels: OnceCell<RgbaImage>,
    converting: RefCell<Option<JoinHandle<RgbaImage>>>,
}

impl Shot {
    /// Native-resolution RGBA pixels (waits for the background conversion
    /// the first time, which has normally finished by then).
    pub fn image(&self) -> &RgbaImage {
        self.pixels.get_or_init(|| {
            let job = self
                .converting
                .borrow_mut()
                .take()
                .expect("conversion started");
            job.join().expect("pixel conversion thread panicked")
        })
    }
}

/// Raised instead of capturing when Screen Recording access is missing:
/// macOS would hand back only the wallpaper, which looks like a grey screen.
#[derive(Debug)]
pub struct NoPermission;

impl std::fmt::Display for NoPermission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Screen Recording permission is missing")
    }
}

impl std::error::Error for NoPermission {}

/// Opens System Settings at Privacy & Security › Screen Recording.
pub fn open_permission_settings() {
    let url = "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";
    if let Err(e) = std::process::Command::new("open").arg(url).spawn() {
        eprintln!("could not open System Settings: {e}");
    }
}

/// What `ScreenCaptureKit` knows about the screens and about this process.
struct Content {
    displays: Vec<(CGDirectDisplayID, Retained<SCDisplay>)>,
    /// Our own entry in the application list, if `ScreenCaptureKit` lists us.
    ours: Option<Retained<SCRunningApplication>>,
}

// SAFETY: these ScreenCaptureKit objects are immutable snapshots; Apple's
// own sample code hands them between queues.
unsafe impl Send for Content {}

/// Types that may cross threads although their bindings are not `Send`.
/// Sealed: only the two types below, both immutable and documented by Apple
/// as safe to use from any thread.
trait CrossThread {}
impl CrossThread for CFRetained<CGImage> {}
impl CrossThread for Retained<SCShareableContent> {}

/// Moves a value out of a completion handler that is not declared `Send`.
struct SendBox<T: CrossThread>(T);
// SAFETY: restricted to `CrossThread` types, which are immutable snapshots
// that Apple allows to be used from any thread.
unsafe impl<T: CrossThread> Send for SendBox<T> {}

impl<T: CrossThread> SendBox<T> {
    /// A method (not `.0`) so closures capture the whole `Send` wrapper.
    fn into_inner(self) -> T {
        self.0
    }
}

static CONTENT: Mutex<Option<Content>> = Mutex::new(None);

fn error_text(err: *mut NSError) -> String {
    // SAFETY: the handler passes either null or a valid NSError.
    match unsafe { err.as_ref() } {
        Some(e) => e.localizedDescription().to_string(),
        None => "unknown error".to_string(),
    }
}

fn fetch_content() -> Result<Content> {
    let (tx, rx) = mpsc::channel();
    let handler = RcBlock::new(move |content: *mut SCShareableContent, err: *mut NSError| {
        // SAFETY: `content` is null or valid for the call; retaining keeps it.
        let r = unsafe { Retained::retain(content) }
            .map(SendBox)
            .ok_or_else(|| error_text(err));
        let _ = tx.send(r);
    });
    // SAFETY: the block matches the handler signature and is kept alive by the callee.
    unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&handler) };
    let content = rx
        .recv_timeout(SCK_TIMEOUT)
        .map_err(|_| anyhow::anyhow!("ScreenCaptureKit did not answer"))?
        .map_err(|e| anyhow::anyhow!("listing screens: {e}"))?
        .0;
    let pid = std::process::id() as i32;
    // SAFETY: plain property reads on live objects.
    let (displays, ours) = unsafe {
        let displays = content
            .displays()
            .iter()
            .map(|d| (d.displayID(), d))
            .collect();
        let ours = content.applications().iter().find(|a| a.processID() == pid);
        (displays, ours)
    };
    Ok(Content { displays, ours })
}

/// Cached content, refreshed when a display we need is not in it (plugged
/// in since the last capture). Holding the lock while fetching makes a
/// capture that arrives during warm-up wait for it instead of repeating it.
fn with_content<R>(ids: &[CGDirectDisplayID], f: impl FnOnce(&Content) -> Result<R>) -> Result<R> {
    let mut guard = CONTENT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let stale = guard.as_ref().is_none_or(|c| {
        ids.iter()
            .any(|id| !c.displays.iter().any(|(d, _)| d == id))
    });
    if stale {
        *guard = Some(fetch_content()?);
    }
    f(guard.as_ref().expect("content was just filled"))
}

/// Native pixel size of a display (`CGDisplayBounds` is in points).
fn pixel_size(id: CGDirectDisplayID, fallback_pt: (usize, usize)) -> (usize, usize) {
    match CGDisplayCopyDisplayMode(id) {
        Some(mode) => (
            CGDisplayMode::pixel_width(Some(&mode)),
            CGDisplayMode::pixel_height(Some(&mode)),
        ),
        None => fallback_pt,
    }
}

type Pending = mpsc::Receiver<Result<SendBox<CFRetained<CGImage>>, String>>;

/// Starts an asynchronous capture of one display (no cursor, none of our
/// own windows) and returns where the result will arrive.
fn start_capture(content: &Content, id: CGDirectDisplayID) -> Result<Pending> {
    let display = &content
        .displays
        .iter()
        .find(|(d, _)| *d == id)
        .context("display not found")?
        .1;
    let b = CGDisplayBounds(id);
    let (w, h) = pixel_size(id, (b.size.width as usize, b.size.height as usize));
    let (tx, rx) = mpsc::channel();
    let handler = RcBlock::new(move |image: *mut CGImage, err: *mut NSError| {
        let r = match NonNull::new(image) {
            // SAFETY: `image` is valid for the call; retaining keeps it.
            Some(p) => Ok(SendBox(unsafe { CFRetained::retain(p) })),
            None => Err(error_text(err)),
        };
        let _ = tx.send(r);
    });
    // SAFETY: standard ScreenCaptureKit calls on live objects; the handler
    // block matches the declared signature.
    unsafe {
        let own: Retained<NSArray<SCRunningApplication>> = match &content.ours {
            Some(app) => NSArray::from_retained_slice(&[app.retain()]),
            None => NSArray::new(),
        };
        let filter = SCContentFilter::initWithDisplay_excludingApplications_exceptingWindows(
            SCContentFilter::alloc(),
            display,
            &own,
            &NSArray::new(),
        );
        let config = SCStreamConfiguration::new();
        config.setWidth(w);
        config.setHeight(h);
        config.setShowsCursor(false);
        SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(
            &filter,
            &config,
            Some(&handler),
        );
    }
    Ok(rx)
}

fn finish_capture(pending: &Pending) -> Result<CFRetained<CGImage>> {
    let r = pending
        .recv_timeout(SCK_TIMEOUT)
        .map_err(|_| anyhow::anyhow!("ScreenCaptureKit did not answer"))?;
    Ok(r.map_err(|e| anyhow::anyhow!("capturing display: {e}"))?.0)
}

fn active_displays() -> Result<Vec<CGDirectDisplayID>> {
    let mut ids: [CGDirectDisplayID; 16] = [0; 16];
    let mut count = 0u32;
    // SAFETY: the buffer holds 16 entries and we pass that capacity.
    unsafe { CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) };
    anyhow::ensure!(count > 0, "no displays found");
    Ok(ids[..count as usize].to_vec())
}

/// The first capture in a process is several times slower than later ones
/// (framework start-up), so the menu-bar app pays that cost at launch: it
/// lists the screens and takes one throw-away capture.
pub fn warm_up() {
    if !CGPreflightScreenCaptureAccess() {
        return;
    }
    std::thread::spawn(|| {
        let main = CGMainDisplayID();
        let _ = with_content(&[main], |c| finish_capture(&start_capture(c, main)?));
    });
}

/// Captures that are in flight, one per display. Starting them and
/// collecting the results are separate steps so the caller can get on with
/// other work (creating its windows) during `ScreenCaptureKit`'s ~60 ms.
pub struct Capture {
    displays: Vec<InFlight>,
}

struct InFlight {
    origin_pt: (i32, i32),
    size_pt: (u32, u32),
    pending: Pending,
}

impl Capture {
    /// Where each display sits: `(origin_pt, size_pt)`, in the order of the
    /// shots [`Capture::finish`] returns.
    pub fn frames(&self) -> Vec<((i32, i32), (u32, u32))> {
        self.displays
            .iter()
            .map(|d| (d.origin_pt, d.size_pt))
            .collect()
    }

    /// Waits for every display's image.
    pub fn finish(self) -> Result<Vec<Shot>> {
        self.displays
            .into_iter()
            .map(|d| {
                let cg = finish_capture(&d.pending)?;
                let converting = start_conversion(&cg);
                Ok(Shot {
                    origin_pt: d.origin_pt,
                    size_pt: d.size_pt,
                    cg,
                    pixels: OnceCell::new(),
                    converting: RefCell::new(Some(converting)),
                })
            })
            .collect()
    }
}

/// Starts capturing every display. Call before any of our windows are visible.
pub fn begin() -> Result<Capture> {
    if !CGPreflightScreenCaptureAccess() {
        // Adds us to the Screen Recording list (and prompts the first time).
        CGRequestScreenCaptureAccess();
        return Err(NoPermission.into());
    }
    let ids = active_displays()?;
    // All displays are captured concurrently.
    let pending = with_content(&ids, |c| {
        ids.iter()
            .map(|&id| start_capture(c, id))
            .collect::<Result<Vec<_>>>()
    })?;
    let displays = ids
        .into_iter()
        .zip(pending)
        .map(|(id, pending)| {
            let b = CGDisplayBounds(id);
            InFlight {
                origin_pt: (b.origin.x.round() as i32, b.origin.y.round() as i32),
                size_pt: (b.size.width.round() as u32, b.size.height.round() as u32),
                pending,
            }
        })
        .collect();
    Ok(Capture { displays })
}

/// Reads the image bytes (a 24 MB copy on a Retina display) and converts
/// them to RGBA on another thread.
fn start_conversion(cg: &CFRetained<CGImage>) -> JoinHandle<RgbaImage> {
    let cg = SendBox(cg.clone());
    std::thread::spawn(move || convert(&cg.into_inner()))
}

/// Never panics: an unreadable image comes out black.
fn convert(cg: &CGImage) -> RgbaImage {
    let w = CGImage::width(Some(cg));
    let h = CGImage::height(Some(cg));
    let row = CGImage::bytes_per_row(Some(cg));
    let alpha = CGImage::alpha_info(Some(cg));
    // ScreenCaptureKit delivers 32-bit little-endian BGRA; handle RGBA too.
    let bgra = CGImage::byte_order_info(Some(cg)) == CGImageByteOrderInfo::Order32Little
        && matches!(
            alpha,
            CGImageAlphaInfo::NoneSkipFirst
                | CGImageAlphaInfo::PremultipliedFirst
                | CGImageAlphaInfo::First
        );
    let bytes = (CGImage::bits_per_pixel(Some(cg)) == 32)
        .then(|| CGImage::data_provider(Some(cg)))
        .flatten()
        .and_then(|p| CGDataProvider::data(Some(&p)))
        .map(|d| d.to_vec());
    match bytes {
        Some(bytes) => to_rgba(&bytes, w, h, row, bgra),
        None => {
            eprintln!("warning: could not read the captured pixels");
            to_rgba(&[], w, h, row, bgra)
        }
    }
}

/// Never panics: rows missing from `bytes` (an unexpected layout) stay black.
fn to_rgba(bytes: &[u8], w: usize, h: usize, row: usize, bgra: bool) -> RgbaImage {
    let mut out = [0, 0, 0, 255].repeat(w * h);
    for y in 0..h {
        let start = y.saturating_mul(row);
        let end = start.saturating_add(w * 4).min(bytes.len());
        let Some(line) = bytes.get(start..end) else {
            break;
        };
        let dst = &mut out[y * w * 4..];
        for (d, p) in dst
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(line.as_chunks::<4>().0)
        {
            *d = if bgra {
                [p[2], p[1], p[0], 255]
            } else {
                [p[0], p[1], p[2], 255]
            };
        }
    }
    RgbaImage::from_raw(w as u32, h as u32, out)
        .unwrap_or_else(|| RgbaImage::new(w as u32, h as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_padded_bgra_rows() {
        // 2×2 image, rows padded to 12 bytes.
        let bytes = [
            1, 2, 3, 0, 4, 5, 6, 0, 9, 9, 9, 9, //
            7, 8, 9, 0, 10, 11, 12, 0, 9, 9, 9, 9,
        ];
        let img = to_rgba(&bytes, 2, 2, 12, true);
        assert_eq!(
            img.as_raw(),
            &[3, 2, 1, 255, 6, 5, 4, 255, 9, 8, 7, 255, 12, 11, 10, 255]
        );
    }

    #[test]
    fn truncated_data_does_not_panic() {
        let img = to_rgba(&[1, 2, 3, 0, 4], 2, 2, 8, true);
        assert_eq!(
            img.as_raw(),
            &[3, 2, 1, 255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255]
        );
    }
}
