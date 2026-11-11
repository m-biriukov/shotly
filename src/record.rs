//! Screen recording to an MP4 file with `ScreenCaptureKit`'s `SCStream` and
//! `SCRecordingOutput` (macOS 15), which encodes H.264 and writes the file
//! itself. Our own windows are part of the picture on purpose: the pointer
//! halo and click ripples (see `highlight`) are windows too.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGDirectDisplayID, CGDisplayBounds};
use objc2_core_media::{CMTime, CMTimeFlags};
use objc2_foundation::{NSArray, NSError, NSObject, NSObjectProtocol, NSString, NSURL, ns_string};
use objc2_screen_capture_kit::{
    SCContentFilter, SCRecordingOutput, SCRecordingOutputConfiguration, SCRecordingOutputDelegate,
    SCStream, SCStreamConfiguration, SCStreamDelegate,
};
use winit::event_loop::EventLoopProxy;

use crate::app::AppEvent;
use crate::capture::{self, SCK_TIMEOUT};
use crate::config::Config;
use crate::highlight::EFFECT_TITLE;
use crate::snap::Frame;

/// What to record: part of one display, optionally one window to follow.
#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub display_id: CGDirectDisplayID,
    /// Global points, top-left origin.
    pub area: Frame,
    /// Window the area was picked from; the recording follows it when it moves.
    pub window: Option<u32>,
}

/// Everything about the stream that stays the same for the whole recording.
#[derive(Clone, Copy)]
struct StreamSettings {
    pixels: (usize, usize),
    fps: u32,
    mic: bool,
}

impl StreamSettings {
    /// A new configuration for recording `source` (display-relative points).
    /// `updateConfiguration` gets a fresh one each time rather than a changed
    /// copy of the one the stream is already reading.
    fn configuration(self, source: CGRect) -> Retained<SCStreamConfiguration> {
        // SAFETY: plain property setters on a freshly created configuration.
        unsafe {
            let c = SCStreamConfiguration::new();
            c.setWidth(self.pixels.0);
            c.setHeight(self.pixels.1);
            c.setSourceRect(source);
            c.setShowsCursor(true);
            c.setMinimumFrameInterval(CMTime {
                value: 1,
                timescale: self.fps as i32,
                flags: CMTimeFlags::Valid,
                epoch: 0,
            });
            c.setQueueDepth(6);
            c.setCapturesAudio(false);
            c.setCaptureMicrophone(self.mic);
            c
        }
    }
}

struct DelegateIvars {
    done: mpsc::Sender<Result<(), String>>,
    proxy: EventLoopProxy<AppEvent>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[ivars = DelegateIvars]
    struct RecDelegate;

    unsafe impl NSObjectProtocol for RecDelegate {}

    unsafe impl SCRecordingOutputDelegate for RecDelegate {
        #[unsafe(method(recordingOutput:didFailWithError:))]
        fn did_fail(&self, _output: &SCRecordingOutput, error: &NSError) {
            let text = error.localizedDescription().to_string();
            self.report(Err(text));
        }

        #[unsafe(method(recordingOutputDidFinishRecording:))]
        fn did_finish(&self, _output: &SCRecordingOutput) {
            self.report(Ok(()));
        }
    }

    // The stream ends on its own when, for instance, the display is unplugged
    // or the permission is revoked.
    unsafe impl SCStreamDelegate for RecDelegate {
        #[unsafe(method(stream:didStopWithError:))]
        fn stream_stopped(&self, _stream: &SCStream, error: &NSError) {
            let text = error.localizedDescription().to_string();
            self.report(Err(text));
        }
    }
);

impl RecDelegate {
    fn new(
        done: mpsc::Sender<Result<(), String>>,
        proxy: EventLoopProxy<AppEvent>,
    ) -> Retained<Self> {
        let this = Self::alloc().set_ivars(DelegateIvars { done, proxy });
        // SAFETY: plain NSObject init of our subclass.
        unsafe { msg_send![super(this), init] }
    }

    fn report(&self, r: Result<(), String>) {
        let iv = self.ivars();
        let _ = iv.done.send(r);
        let _ = iv.proxy.send_event(AppEvent::RecordingDone);
    }
}

pub struct Recording {
    stream: Retained<SCStream>,
    settings: StreamSettings,
    _output: Retained<SCRecordingOutput>,
    _delegate: Retained<RecDelegate>,
    done_tx: mpsc::Sender<Result<(), String>>,
    done: mpsc::Receiver<Result<(), String>>,
    proxy: EventLoopProxy<AppEvent>,
    pub path: PathBuf,
    /// The microphone could not be used, so the file has no sound.
    pub mic_unavailable: bool,
    /// Display frame and the size of the recorded area, global points.
    display: (f64, f64, f64, f64),
    size: (f64, f64),
    last_origin: (f64, f64),
    pub window: Option<u32>,
    stopping: bool,
}

/// True when macOS has been told to refuse (or forbids) microphone access,
/// so asking for it would fail. "Not yet asked" is not blocked: the first
/// recording with the microphone triggers the system prompt.
fn microphone_blocked() -> bool {
    // SAFETY: reads an AVFoundation constant and the current permission state;
    // neither has side effects.
    let status = unsafe {
        AVMediaTypeAudio.map(|kind| AVCaptureDevice::authorizationStatusForMediaType(kind))
    };
    matches!(
        status,
        Some(AVAuthorizationStatus::Denied | AVAuthorizationStatus::Restricted)
    )
}

/// Starts recording `target` into `path`. If the microphone is wanted but
/// cannot be used (permission denied), records without it and says so in
/// [`Recording::mic_unavailable`].
pub fn start(
    target: &Target,
    cfg: &Config,
    path: &std::path::Path,
    proxy: &EventLoopProxy<AppEvent>,
) -> Result<Recording> {
    let dir = path.parent().unwrap_or(std::path::Path::new("."));
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let blocked = cfg.record_mic && microphone_blocked();
    let with_mic = cfg.record_mic && !blocked;
    match Recording::begin(target, cfg, path, proxy, with_mic) {
        Ok(mut rec) => {
            rec.mic_unavailable = blocked;
            Ok(rec)
        }
        Err(e) if with_mic => {
            eprintln!("recording with microphone failed ({e:#}); retrying without it");
            // The failed attempt may still be shutting down and may have
            // created the file, so the retry gets a name of its own.
            let name = path.file_name().map_or_else(
                || "recording.mp4".to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            let retry = unique_path(dir, &name)?;
            let mut rec = Recording::begin(target, cfg, &retry, proxy, false)?;
            rec.mic_unavailable = true;
            Ok(rec)
        }
        Err(e) => Err(e),
    }
}

/// How many "name (2).ext", "name (3).ext"… candidates are tried.
const MAX_NAME_TRIES: u32 = 1000;

/// `name` in `dir`, or "name (2).ext", "name (3).ext"… if taken, so a new
/// recording never overwrites an older one. (A file that appears between this
/// check and the start of the recording is not detected: SCK creates the file
/// itself, so it cannot be claimed first.)
pub fn unique_path(dir: &std::path::Path, name: &str) -> Result<PathBuf> {
    unique_path_within(dir, name, MAX_NAME_TRIES)
}

fn unique_path_within(dir: &std::path::Path, name: &str, tries: u32) -> Result<PathBuf> {
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, "mp4"));
    (1..=tries)
        .map(|n| {
            if n == 1 {
                dir.join(name)
            } else {
                dir.join(format!("{stem} ({n}).{ext}"))
            }
        })
        .find(|p| !p.exists())
        .with_context(|| format!("{tries} files named like {name} already exist"))
}

/// Makes the finished video readable by its owner only, like screenshots.
pub fn make_private(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
        eprintln!("could not restrict {}: {e}", path.display());
    }
}

/// Even pixel count (H.264 needs even sizes) for `pt` points at `scale`.
fn pixels(pt: f64, scale: f64) -> usize {
    ((pt * scale).round() as usize & !1).max(2)
}

impl Recording {
    fn begin(
        target: &Target,
        cfg: &Config,
        path: &std::path::Path,
        proxy: &EventLoopProxy<AppEvent>,
        mic: bool,
    ) -> Result<Self> {
        let id = target.display_id;
        let b = CGDisplayBounds(id);
        let display = (b.origin.x, b.origin.y, b.size.width, b.size.height);
        let (px_w, _) = capture::pixel_size(id, (b.size.width as usize, b.size.height as usize));
        let scale = if cfg.record_retina {
            px_w as f64 / b.size.width.max(1.0)
        } else {
            1.0
        };
        let area = target.area;
        let (w, h) = (area.w.min(display.2), area.h.min(display.3));
        let origin = clamp_origin((area.x, area.y), (w, h), display);

        // Windows of ours that are open now (a notice, say) stay out of the
        // video; the halo and ripples (see `highlight`) stay in. The app does
        // not open Settings while recording, so nothing else can appear.
        let hidden = capture::own_windows_except(EFFECT_TITLE).unwrap_or_else(|e| {
            eprintln!("could not list our windows: {e:#}");
            NSArray::new()
        });
        let filter = capture::with_display(id, |d| {
            // SAFETY: standard ScreenCaptureKit call on a live display.
            Ok(unsafe {
                SCContentFilter::initWithDisplay_excludingWindows(
                    SCContentFilter::alloc(),
                    d,
                    &hidden,
                )
            })
        })?;

        let settings = StreamSettings {
            pixels: (pixels(w, scale), pixels(h, scale)),
            fps: cfg.record_fps,
            mic,
        };
        let config = settings.configuration(source_rect(origin, (w, h), display));

        let (done_tx, done) = mpsc::channel();
        let delegate = RecDelegate::new(done_tx.clone(), proxy.clone());
        let delegate_proto = ProtocolObject::from_ref(&*delegate);
        // SAFETY: plain property setters on a new configuration; the URL and
        // the two constant strings outlive the calls.
        let output = unsafe {
            let oc = SCRecordingOutputConfiguration::new();
            let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
            oc.setOutputURL(&url);
            oc.setVideoCodecType(ns_string!("avc1"));
            oc.setOutputFileType(ns_string!("public.mpeg-4"));
            SCRecordingOutput::initWithConfiguration_delegate(
                SCRecordingOutput::alloc(),
                &oc,
                delegate_proto,
            )
        };
        let delegate_proto = ProtocolObject::from_ref(&*delegate);
        // SAFETY: filter and configuration are live; the delegate is kept
        // alive by `Recording` for as long as the stream may call it.
        let stream = unsafe {
            SCStream::initWithFilter_configuration_delegate(
                SCStream::alloc(),
                &filter,
                &config,
                Some(delegate_proto),
            )
        };
        // SAFETY: `output` is a fully configured recording output that this
        // stream does not have yet; it is kept alive by `Recording`.
        unsafe { stream.addRecordingOutput_error(&output) }.map_err(|e| {
            anyhow::anyhow!("adding recording output: {}", e.localizedDescription())
        })?;

        let (tx, rx) = mpsc::channel();
        let handler = RcBlock::new(move |err: *mut NSError| {
            let r = if err.is_null() {
                Ok(())
            } else {
                Err(capture::error_text(err))
            };
            let _ = tx.send(r);
        });
        // SAFETY: the block matches the handler signature.
        unsafe { stream.startCaptureWithCompletionHandler(Some(&handler)) };
        let started = rx
            .recv_timeout(SCK_TIMEOUT)
            .map_err(|_| anyhow::anyhow!("ScreenCaptureKit did not start the recording"))
            .and_then(|r| r.map_err(|e| anyhow::anyhow!("starting recording: {e}")));
        if let Err(e) = started {
            // A start that is merely late must not run on with nobody to stop it.
            // SAFETY: stopping a stream we created; no handler is needed.
            unsafe { stream.stopCaptureWithCompletionHandler(None) };
            return Err(e);
        }

        Ok(Self {
            stream,
            settings,
            _output: output,
            _delegate: delegate,
            done_tx,
            done,
            proxy: proxy.clone(),
            path: path.to_path_buf(),
            mic_unavailable: false,
            display,
            size: (w, h),
            last_origin: origin,
            window: target.window,
            stopping: false,
        })
    }

    /// Moves the recorded area to `frame`'s corner (the window it follows
    /// moved). The size stays what it was when the recording started.
    pub fn follow(&mut self, frame: &Frame) {
        let origin = clamp_origin((frame.x, frame.y), self.size, self.display);
        if (origin.0 - self.last_origin.0).abs() < 0.5
            && (origin.1 - self.last_origin.1).abs() < 0.5
        {
            return;
        }
        self.last_origin = origin;
        let config = self
            .settings
            .configuration(source_rect(origin, self.size, self.display));
        // SAFETY: update call on a live stream with a configuration it has not seen.
        unsafe {
            self.stream
                .updateConfiguration_completionHandler(&config, None);
        }
    }

    /// Where the recorded area is now (global points).
    pub fn area(&self) -> Frame {
        Frame {
            id: self.window,
            x: self.last_origin.0,
            y: self.last_origin.1,
            w: self.size.0,
            h: self.size.1,
        }
    }

    /// Asks the stream to stop; the finished file is reported through
    /// [`Recording::poll`] after an `AppEvent::RecordingDone`.
    pub fn stop(&mut self) {
        if std::mem::replace(&mut self.stopping, true) {
            return;
        }
        let tx = self.done_tx.clone();
        let proxy = self.proxy.clone();
        // Success is reported by the recording output once the file is
        // complete; only a failure to stop is reported from here.
        let handler = RcBlock::new(move |err: *mut NSError| {
            if !err.is_null() {
                let _ = tx.send(Err(capture::error_text(err)));
                let _ = proxy.send_event(AppEvent::RecordingDone);
            }
        });
        // SAFETY: the block matches the handler signature.
        unsafe { self.stream.stopCaptureWithCompletionHandler(Some(&handler)) };
    }

    pub fn is_stopping(&self) -> bool {
        self.stopping
    }

    /// The outcome once the recording has ended (stopped, or failed on its own).
    pub fn poll(&self) -> Option<Result<PathBuf, String>> {
        let r = self.done.try_recv().ok()?;
        Some(r.map(|()| self.path.clone()))
    }

    /// Stops and waits (at most `timeout`) for the file; used when quitting.
    pub fn stop_and_wait(&mut self, timeout: Duration) -> Result<PathBuf, String> {
        self.stop();
        match self.done.recv_timeout(timeout) {
            Ok(r) => r.map(|()| self.path.clone()),
            Err(_) => Err("the recording did not finish in time".into()),
        }
    }
}

/// Top-left corner for an area `size` large, kept inside `display`.
fn clamp_origin(origin: (f64, f64), size: (f64, f64), display: (f64, f64, f64, f64)) -> (f64, f64) {
    let x = origin
        .0
        .clamp(display.0, (display.0 + display.2 - size.0).max(display.0));
    let y = origin
        .1
        .clamp(display.1, (display.1 + display.3 - size.1).max(display.1));
    (x, y)
}

/// The area as `SCStreamConfiguration.sourceRect`: points relative to the
/// display's top-left corner.
fn source_rect(origin: (f64, f64), size: (f64, f64), display: (f64, f64, f64, f64)) -> CGRect {
    CGRect::new(
        CGPoint::new(origin.0 - display.0, origin.1 - display.1),
        CGSize::new(size.0, size.1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_out_of_names_is_an_error_not_an_overwrite() {
        let dir = std::env::temp_dir().join(format!("shotly-names-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.mp4"), b"x").unwrap();
        std::fs::write(dir.join("a (2).mp4"), b"x").unwrap();
        assert!(unique_path_within(&dir, "a.mp4", 2).is_err());
        assert_eq!(
            unique_path_within(&dir, "a.mp4", 3).unwrap(),
            dir.join("a (3).mp4")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn existing_videos_are_never_overwritten() {
        let dir = std::env::temp_dir().join(format!("shotly-rec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_path(&dir, "a.mp4").unwrap(), dir.join("a.mp4"));
        std::fs::write(dir.join("a.mp4"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "a.mp4").unwrap(), dir.join("a (2).mp4"));
        std::fs::write(dir.join("a (2).mp4"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "a.mp4").unwrap(), dir.join("a (3).mp4"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pixel_sizes_are_even_and_at_least_two() {
        assert_eq!(pixels(101.0, 2.0), 202);
        assert_eq!(pixels(100.5, 2.0), 200);
        assert_eq!(pixels(101.0, 1.0), 100);
        assert_eq!(pixels(0.0, 2.0), 2);
    }

    #[test]
    fn origin_stays_inside_the_display() {
        let display = (-1000.0, 0.0, 1000.0, 800.0);
        assert_eq!(
            clamp_origin((-1200.0, -5.0), (300.0, 200.0), display),
            (-1000.0, 0.0)
        );
        assert_eq!(
            clamp_origin((-100.0, 700.0), (300.0, 200.0), display),
            (-300.0, 600.0)
        );
        assert_eq!(
            clamp_origin((-500.0, 100.0), (300.0, 200.0), display),
            (-500.0, 100.0)
        );
    }

    #[test]
    fn source_rect_is_relative_to_the_display() {
        let r = source_rect(
            (-900.0, 50.0),
            (300.0, 200.0),
            (-1000.0, 0.0, 1000.0, 800.0),
        );
        assert_eq!((r.origin.x, r.origin.y), (100.0, 50.0));
        assert_eq!((r.size.width, r.size.height), (300.0, 200.0));
    }
}
