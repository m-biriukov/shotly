//! Recording inside the menu-bar app: the 3-2-1 countdown, starting, following
//! a window, stopping, and what happens when a recording ends or fails.

use std::time::{Duration, Instant};

use winit::event_loop::ActiveEventLoop;

use super::flow::{Cmd, Input};
use super::{App, TOAST_LONG, saved_notice};
use crate::highlight::{Countdown, Effects, Outline};
use crate::record::{self, Recording, Target};
use crate::toast::Notice;
use crate::tray::{recording_icon_image, tray_icon_image};
use crate::{macos, snap};

/// How often a recording checks whether the window it follows has moved.
const FOLLOW_EVERY: Duration = Duration::from_millis(100);

/// How long quitting waits for a recording to be finished into a playable file.
const QUIT_WAIT: Duration = Duration::from_secs(5);

const COUNTDOWN_SECONDS: u32 = 3;

/// How long a stopped recording may take to report before it counts as failed.
const STOP_WAIT: Duration = Duration::from_secs(10);

/// The 3-2-1 before a recording starts.
pub(super) struct Counting {
    pub(super) countdown: Countdown,
    pub(super) target: Target,
    /// Kept alive so the frame shows while counting.
    pub(super) outline: Option<Outline>,
}

/// A recording that is no longer on screen but not yet done with.
pub(super) struct Finishing {
    pub(super) rec: Recording,
    pub(super) give_up: Instant,
    /// Tell the user if it turns out to have been saved after all.
    pub(super) report_late_save: bool,
}

/// A recording in progress (or being finished).
pub(super) struct Active {
    pub(super) rec: Recording,
    /// Halo and ripples; dropped as soon as the recording is stopped.
    pub(super) effects: Option<Effects>,
    pub(super) outline: Option<Outline>,
    pub(super) started: Instant,
    /// Whole seconds shown in the menu bar.
    pub(super) shown_secs: Option<u64>,
    pub(super) next_follow: Instant,
    /// Set when stopping: past it the recording is given up on.
    pub(super) stop_deadline: Option<Instant>,
    /// Frame (global points) of the display, for the "Saved" notice.
    pub(super) display: macos::Frame,
}

impl App {
    /// The area was picked: count down (if wanted), then record. `cmds` is
    /// what the flow decided: [`Cmd::BeginCountdown`] or [`Cmd::StartStream`].
    pub(super) fn begin_recording(&mut self, el: &ActiveEventLoop, target: Target, cmds: &[Cmd]) {
        // Nothing of ours may be on screen but the recording's own overlays.
        self.toast = None;
        if let Some(s) = &self.settings {
            s.close();
        }
        let outline = self
            .cfg
            .record_outline
            .then(|| Outline::new(el, target.area))
            .and_then(|r| r.map_err(|e| eprintln!("outline failed: {e:#}")).ok());
        if cmds.contains(&Cmd::StartStream) {
            return self.start_recording(el, target, outline);
        }
        let now = Instant::now();
        let a = target.area;
        let centre = (a.x + a.w / 2.0, a.y + a.h / 2.0);
        match Countdown::new(el, centre, COUNTDOWN_SECONDS, now) {
            Ok(countdown) => {
                self.counting = Some(Counting {
                    countdown,
                    target,
                    outline,
                });
                self.set_recording_ui(true, None);
            }
            Err(e) => {
                eprintln!("countdown failed: {e:#}");
                // No countdown to wait for: on with the recording.
                self.flow
                    .step(Input::CountdownFinished, self.cfg.record_countdown);
                self.start_recording(el, target, outline);
            }
        }
    }

    pub(super) fn start_recording(
        &mut self,
        el: &ActiveEventLoop,
        target: Target,
        outline: Option<Outline>,
    ) {
        let display = macos::display_frame(target.display_id);
        let path = match record::unique_path(
            &self.cfg.save_dir(),
            &self.cfg.video_name(chrono::Local::now()),
        ) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("could not start recording: {e:#}");
                self.set_recording_ui(false, None);
                self.flow
                    .step(Input::StartResult { ok: false }, self.cfg.record_countdown);
                let notice =
                    Notice::new("Could not start recording", TOAST_LONG).detail(format!("{e:#}"));
                return self.notify(el, Some(display), notice.important());
            }
        };
        match record::start(&target, &self.cfg, &path, &self.proxy) {
            Ok(rec) => {
                let now = Instant::now();
                self.recording = Some(Active {
                    rec,
                    effects: Some(Effects::new(el, &self.cfg)),
                    outline,
                    started: now,
                    shown_secs: None,
                    next_follow: now,
                    stop_deadline: None,
                    display,
                });
                self.flow
                    .step(Input::StartResult { ok: true }, self.cfg.record_countdown);
                self.set_recording_ui(true, Some(0));
            }
            Err(e) => {
                eprintln!("could not start recording: {e:#}");
                self.flow
                    .step(Input::StartResult { ok: false }, self.cfg.record_countdown);
                self.set_recording_ui(false, None);
                let notice =
                    Notice::new("Could not start recording", TOAST_LONG).detail(format!("{e:#}"));
                self.notify(el, Some(display), notice.important());
            }
        }
    }

    /// The record shortcut or menu item: start, or stop what is running.
    pub(super) fn toggle_recording(&mut self, el: &ActiveEventLoop) {
        let cmds = self
            .flow
            .step(Input::RecordShortcut, self.cfg.record_countdown);
        for cmd in cmds {
            match cmd {
                Cmd::OpenPicker(mode) => self.open_picker(el, mode),
                Cmd::CancelCountdown => {
                    self.counting = None;
                    self.set_recording_ui(false, None);
                }
                Cmd::StopStream => {
                    if let Some(active) = &mut self.recording {
                        // The halo and ripples must not outlive the video.
                        active.effects = None;
                        active.outline = None;
                        active.rec.stop();
                        active.stop_deadline = Some(Instant::now() + STOP_WAIT);
                    }
                }
                _ => {}
            }
        }
    }

    /// The recording ended: show the result and reset the menu bar.
    pub(super) fn recording_done(&mut self, el: &ActiveEventLoop) {
        let Some(active) = &self.recording else {
            return;
        };
        let Some(result) = active.rec.poll() else {
            return;
        };
        let Some(active) = self.recording.take() else {
            return;
        };
        self.flow
            .step(Input::StreamEnded, self.cfg.record_countdown);
        self.set_recording_ui(false, None);
        let display = Some(active.display);
        match result {
            Ok(path) => {
                record::make_private(&path);
                let mic_lost = active.rec.mic_unavailable;
                let title = if mic_lost {
                    "Saved without sound — click to show in Finder"
                } else {
                    "Saved — click to show in Finder"
                };
                let mut notice = saved_notice(title, path);
                if mic_lost {
                    // Important: not having sound is something to hear about.
                    notice = notice.important();
                }
                self.notify(el, display, notice);
            }
            Err(e) => self.abandon(el, active, e, false),
        }
    }

    /// Drops recordings that have reported (or timed out); one that was only
    /// slow to finish gets its "Saved" notice after all.
    pub(super) fn reap_finishing(&mut self, el: &ActiveEventLoop, now: Instant) {
        let mut saved = Vec::new();
        self.finishing.retain(|f| match f.rec.poll() {
            Some(Ok(path)) => {
                if f.report_late_save {
                    saved.push(path);
                }
                false
            }
            Some(Err(_)) => false,
            None => now < f.give_up,
        });
        for path in saved {
            record::make_private(&path);
            let notice = saved_notice("Saved after all — click to show in Finder", path);
            self.notify(el, None, notice);
        }
    }

    /// Gives up on a recording that failed (or never reported): tells the
    /// user, and keeps it alive while it is stopped so no stream or delegate
    /// is freed while still in use.
    /// `late_save`: the file may still finish after all (the stop was just
    /// slow); say so if it does.
    pub(super) fn abandon(
        &mut self,
        el: &ActiveEventLoop,
        active: Active,
        why: String,
        late_save: bool,
    ) {
        eprintln!("recording failed: {why}");
        let display = Some(active.display);
        let mut rec = active.rec;
        rec.stop();
        self.finishing.push(Finishing {
            rec,
            give_up: Instant::now() + STOP_WAIT,
            report_late_save: late_save,
        });
        let notice = Notice::new("Recording failed", TOAST_LONG).detail(why);
        self.notify(el, display, notice.important());
    }

    /// Red menu-bar icon (and elapsed time) while counting down or recording.
    pub(super) fn set_recording_ui(&mut self, on: bool, elapsed: Option<u64>) {
        let Some(tray) = &self.tray else { return };
        if on {
            let _ = tray.set_icon(Some(recording_icon_image()));
            tray.set_title(elapsed.map(format_elapsed));
        } else {
            let _ = tray.set_icon_templated(Some(tray_icon_image()));
            tray.set_title(None::<&str>);
        }
        if let Some(m) = &self.menu {
            m.record.set_text(if on {
                "Stop Recording"
            } else {
                "Record Screen"
            });
        }
    }

    /// Per-frame work while counting down or recording. Returns when to
    /// be woken next.
    pub(super) fn tick_recording(&mut self, el: &ActiveEventLoop, now: Instant) -> Option<Instant> {
        let mut wake = None;
        if let Some(c) = &mut self.counting {
            match c.countdown.update(now) {
                Some(at) => wake = Some(at),
                None => {
                    if let Some(Counting {
                        mut countdown,
                        target,
                        outline,
                    }) = self.counting.take()
                    {
                        // Off screen before the stream starts (which takes
                        // far longer than hiding a window).
                        countdown.hide();
                        self.flow
                            .step(Input::CountdownFinished, self.cfg.record_countdown);
                        self.start_recording(el, target, outline);
                    }
                }
            }
        }
        self.recording_done(el);
        self.reap_finishing(el, now);
        if overdue(self.recording.as_ref().and_then(|a| a.stop_deadline), now)
            && let Some(active) = self.recording.take()
        {
            self.flow
                .step(Input::StopOverdue, self.cfg.record_countdown);
            self.set_recording_ui(false, None);
            self.abandon(
                el,
                active,
                "the recording did not finish in time".into(),
                true,
            );
        }
        if let Some(a) = &mut self.recording {
            let mut next = now + FOLLOW_EVERY;
            if let Some(fx) = &mut a.effects
                && let Some(at) = fx.tick(el, now)
            {
                next = at;
            }
            let secs = now.saturating_duration_since(a.started).as_secs();
            if a.shown_secs != Some(secs) && !a.rec.is_stopping() {
                a.shown_secs = Some(secs);
                if let Some(tray) = &self.tray {
                    tray.set_title(Some(format_elapsed(secs)));
                }
            }
            if now >= a.next_follow {
                a.next_follow = now + FOLLOW_EVERY;
                if let Some(id) = a.rec.window
                    && let Some(frame) = snap::frame_of(id)
                {
                    a.rec.follow(&frame);
                    if let Some(o) = &mut a.outline {
                        o.set_area(a.rec.area());
                    }
                }
            }
            wake = Some(wake.map_or(next, |w| w.min(next)));
        }
        wake
    }

    /// Quits; a recording in progress is finished into a playable file first.
    pub(super) fn quit(&mut self, el: &ActiveEventLoop) {
        let cmds = self.flow.step(Input::Quit, self.cfg.record_countdown);
        if cmds.contains(&Cmd::StopAndWait)
            && let Some(mut a) = self.recording.take()
        {
            a.effects = None;
            a.outline = None;
            match a.rec.stop_and_wait(QUIT_WAIT) {
                Ok(p) => println!("Saved {}", p.display()),
                Err(e) => eprintln!("recording not finished cleanly: {e}"),
            }
        }
        el.exit();
    }
}

/// A stopped recording that has not reported by its `deadline` is given up on.
fn overdue(deadline: Option<Instant>, now: Instant) -> bool {
    deadline.is_some_and(|d| now >= d)
}

/// "0:07", "12:30": the time shown next to the menu-bar icon.
fn format_elapsed(secs: u64) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stop_is_overdue_only_after_its_deadline() {
        let now = Instant::now();
        assert!(!overdue(None, now));
        assert!(!overdue(Some(now + Duration::from_secs(1)), now));
        assert!(overdue(Some(now), now));
        assert!(overdue(Some(now), now + Duration::from_secs(1)));
    }

    #[test]
    fn elapsed_time_is_minutes_and_seconds() {
        assert_eq!(format_elapsed(0), "0:00");
        assert_eq!(format_elapsed(7), "0:07");
        assert_eq!(format_elapsed(754), "12:34");
    }
}
