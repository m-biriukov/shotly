//! Which of the app's modes is on (picking an area, counting down, recording…)
//! and what each event does in it. No windows, no timers, no `ScreenCaptureKit`:
//! [`Flow::step`] takes an [`Input`] and says in [`Cmd`]s what the app must do,
//! so every transition can be unit tested. The app (`app.rs`,
//! `app/recording.rs`) carries the commands out and reports back what
//! happened.

use crate::overlay::Mode;

/// What the app is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Nothing; the menu-bar icon is the plain one.
    Idle,
    /// The freeze-frame picker is open.
    Picking(Mode),
    /// An area was picked; the 3-2-1 is running.
    Counting,
    /// The stream is being started (it takes a moment, and may fail).
    Starting,
    Recording,
    /// Stop was asked for; waiting for the file to be finished.
    Stopping,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    /// The record shortcut, or the menu item while a recording is running.
    RecordShortcut,
    /// The capture shortcut (`Mode::Screenshot`), or the delayed menu item.
    CaptureRequested(Mode),
    /// The picker ended. `picked`: an area to record was chosen.
    PickerClosed { picked: bool },
    /// The countdown is over (or could not be shown).
    CountdownFinished,
    /// The stream did, or did not, start.
    StartResult { ok: bool },
    /// The recording ended: finished, failed, or the stream stopped on its own.
    StreamEnded,
    /// A stop that was asked for took too long.
    StopOverdue,
    /// The app is quitting.
    Quit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    OpenPicker(Mode),
    /// "Recording in progress": a screenshot was asked for during a recording.
    ShowBusyNotice,
    BeginCountdown,
    CancelCountdown,
    StartStream,
    StopStream,
    /// Show how the recording ended and reset the menu bar.
    FinishRecording,
    /// Give up on a stop that never reported.
    AbandonRecording,
    /// Quit while recording: stop and wait (briefly) for a playable file.
    StopAndWait,
}

#[derive(Debug)]
pub struct Flow {
    phase: Phase,
}

impl Flow {
    pub fn new() -> Self {
        Self { phase: Phase::Idle }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// A recording is being prepared or is running, so the record shortcut
    /// means "stop" and a screenshot is refused.
    pub fn running(&self) -> bool {
        matches!(
            self.phase,
            Phase::Counting | Phase::Starting | Phase::Recording | Phase::Stopping
        )
    }

    /// Applies `input` (`countdown`: the 3-2-1 is turned on in Settings) and
    /// returns what the app has to do.
    #[allow(
        clippy::match_same_arms,
        reason = "one arm per (phase, event) keeps this a readable transition table"
    )]
    pub fn step(&mut self, input: Input, countdown: bool) -> Vec<Cmd> {
        use Phase::{Counting, Idle, Picking, Recording, Starting, Stopping};
        match (self.phase, input) {
            // ---- asking for a picker ----
            (Idle, Input::CaptureRequested(mode)) => {
                self.go(Picking(mode), [Cmd::OpenPicker(mode)])
            }
            (Idle, Input::RecordShortcut) => {
                self.go(Picking(Mode::Record), [Cmd::OpenPicker(Mode::Record)])
            }
            // The picker is open already.
            (Picking(_), Input::CaptureRequested(_) | Input::RecordShortcut) => vec![],
            // A screenshot would put its overlay into the video; a second
            // recording request while one runs is the same as the shortcut.
            (
                Counting | Starting | Recording | Stopping,
                Input::CaptureRequested(Mode::Screenshot),
            ) => {
                vec![Cmd::ShowBusyNotice]
            }
            (Counting | Starting | Recording | Stopping, Input::CaptureRequested(Mode::Record)) => {
                vec![]
            }

            // ---- the record shortcut while running ----
            (Counting, Input::RecordShortcut) => self.go(Idle, [Cmd::CancelCountdown]),
            (Recording, Input::RecordShortcut) => self.go(Stopping, [Cmd::StopStream]),
            (Starting | Stopping, Input::RecordShortcut) => vec![],

            // ---- the picker ends ----
            (Picking(Mode::Record), Input::PickerClosed { picked: true }) => {
                if countdown {
                    self.go(Counting, [Cmd::BeginCountdown])
                } else {
                    self.go(Starting, [Cmd::StartStream])
                }
            }
            (Picking(_), Input::PickerClosed { .. }) => self.go(Idle, []),

            // ---- counting down and starting ----
            (Counting, Input::CountdownFinished) => self.go(Starting, [Cmd::StartStream]),
            (Starting, Input::StartResult { ok: true }) => self.go(Recording, []),
            (Starting, Input::StartResult { ok: false }) => self.go(Idle, []),

            // ---- the end ----
            (Recording | Stopping, Input::StreamEnded) => self.go(Idle, [Cmd::FinishRecording]),
            (Stopping, Input::StopOverdue) => self.go(Idle, [Cmd::AbandonRecording]),
            (Counting, Input::Quit) => self.go(Idle, [Cmd::CancelCountdown]),
            (Recording | Stopping, Input::Quit) => self.go(Idle, [Cmd::StopAndWait]),

            // Anything else (a late event for something already over) changes nothing.
            _ => vec![],
        }
    }

    fn go<const N: usize>(&mut self, phase: Phase, cmds: [Cmd; N]) -> Vec<Cmd> {
        self.phase = phase;
        cmds.to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Cmd::*;
    use Input::*;
    use Phase::*;

    /// A flow brought to `phase` by real inputs.
    fn at(phase: Phase) -> Flow {
        let path: &[Input] = match phase {
            Idle => &[],
            Picking(Mode::Screenshot) => &[CaptureRequested(Mode::Screenshot)],
            Picking(Mode::Record) => &[RecordShortcut],
            Counting => &[RecordShortcut, PickerClosed { picked: true }],
            Starting => &[
                RecordShortcut,
                PickerClosed { picked: true },
                CountdownFinished,
            ],
            Recording => &[
                RecordShortcut,
                PickerClosed { picked: true },
                CountdownFinished,
                StartResult { ok: true },
            ],
            Stopping => &[
                RecordShortcut,
                PickerClosed { picked: true },
                CountdownFinished,
                StartResult { ok: true },
                RecordShortcut,
            ],
        };
        let mut f = Flow::new();
        for &i in path {
            f.step(i, true);
        }
        assert_eq!(f.phase(), phase, "setup");
        f
    }

    fn run(phase: Phase, input: Input, countdown: bool) -> (Phase, Vec<Cmd>) {
        let mut f = at(phase);
        let cmds = f.step(input, countdown);
        (f.phase(), cmds)
    }

    #[test]
    fn a_new_flow_is_idle_and_not_running() {
        let f = Flow::new();
        assert_eq!(f.phase(), Idle);
        assert!(!f.running());
    }

    #[test]
    fn running_covers_everything_from_the_countdown_to_the_stop() {
        for p in [Counting, Starting, Recording, Stopping] {
            assert!(at(p).running(), "{p:?}");
        }
        for p in [Idle, Picking(Mode::Screenshot), Picking(Mode::Record)] {
            assert!(!at(p).running(), "{p:?}");
        }
    }

    #[test]
    fn the_shortcuts_open_the_right_picker_when_idle() {
        assert_eq!(
            run(Idle, CaptureRequested(Mode::Screenshot), true),
            (
                Picking(Mode::Screenshot),
                vec![OpenPicker(Mode::Screenshot)]
            )
        );
        assert_eq!(
            run(Idle, CaptureRequested(Mode::Record), true),
            (Picking(Mode::Record), vec![OpenPicker(Mode::Record)])
        );
        assert_eq!(
            run(Idle, RecordShortcut, true),
            (Picking(Mode::Record), vec![OpenPicker(Mode::Record)])
        );
    }

    #[test]
    fn a_second_request_while_the_picker_is_open_does_nothing() {
        for open in [Mode::Screenshot, Mode::Record] {
            for input in [
                CaptureRequested(Mode::Screenshot),
                CaptureRequested(Mode::Record),
                RecordShortcut,
            ] {
                assert_eq!(
                    run(Picking(open), input, true),
                    (Picking(open), vec![]),
                    "{open:?} {input:?}"
                );
            }
        }
    }

    #[test]
    fn a_screenshot_during_a_recording_is_refused_with_a_notice() {
        for p in [Counting, Starting, Recording, Stopping] {
            assert_eq!(
                run(p, CaptureRequested(Mode::Screenshot), true),
                (p, vec![ShowBusyNotice]),
                "{p:?}"
            );
            assert_eq!(
                run(p, CaptureRequested(Mode::Record), true),
                (p, vec![]),
                "{p:?}"
            );
        }
    }

    #[test]
    fn the_record_shortcut_cancels_a_countdown_and_stops_a_recording() {
        assert_eq!(
            run(Counting, RecordShortcut, true),
            (Idle, vec![CancelCountdown])
        );
        assert_eq!(
            run(Recording, RecordShortcut, true),
            (Stopping, vec![StopStream])
        );
        // Pressing it again while the stop is pending, or the stream is
        // starting, changes nothing.
        assert_eq!(run(Stopping, RecordShortcut, true), (Stopping, vec![]));
        assert_eq!(run(Starting, RecordShortcut, true), (Starting, vec![]));
    }

    #[test]
    fn a_picked_area_counts_down_or_starts_at_once() {
        assert_eq!(
            run(Picking(Mode::Record), PickerClosed { picked: true }, true),
            (Counting, vec![BeginCountdown])
        );
        assert_eq!(
            run(Picking(Mode::Record), PickerClosed { picked: true }, false),
            (Starting, vec![StartStream])
        );
    }

    #[test]
    fn a_cancelled_or_screenshot_picker_goes_back_to_idle() {
        for (open, picked) in [
            (Mode::Record, false),
            (Mode::Screenshot, false),
            // A screenshot picker never yields an area to record.
            (Mode::Screenshot, true),
        ] {
            assert_eq!(
                run(Picking(open), PickerClosed { picked }, true),
                (Idle, vec![]),
                "{open:?} {picked}"
            );
        }
    }

    #[test]
    fn the_countdown_leads_to_the_stream_and_the_stream_to_recording() {
        assert_eq!(
            run(Counting, CountdownFinished, true),
            (Starting, vec![StartStream])
        );
        assert_eq!(
            run(Starting, StartResult { ok: true }, true),
            (Recording, vec![])
        );
    }

    #[test]
    fn a_stream_that_does_not_start_leaves_nothing_running() {
        let (phase, cmds) = run(Starting, StartResult { ok: false }, true);
        assert_eq!((phase, cmds), (Idle, vec![]));
        // So a new recording can be started right away.
        let mut f = at(Starting);
        f.step(StartResult { ok: false }, true);
        assert_eq!(f.step(RecordShortcut, true), vec![OpenPicker(Mode::Record)]);
    }

    #[test]
    fn a_recording_that_ends_is_finished_whether_stopped_or_not() {
        // Stopped by the user, or the stream died on its own.
        for p in [Recording, Stopping] {
            assert_eq!(
                run(p, StreamEnded, true),
                (Idle, vec![FinishRecording]),
                "{p:?}"
            );
        }
    }

    #[test]
    fn only_a_pending_stop_can_be_overdue() {
        assert_eq!(
            run(Stopping, StopOverdue, true),
            (Idle, vec![AbandonRecording])
        );
        for p in [Idle, Picking(Mode::Record), Counting, Starting, Recording] {
            assert_eq!(run(p, StopOverdue, true), (p, vec![]), "{p:?}");
        }
    }

    #[test]
    fn quitting_stops_what_is_running() {
        assert_eq!(run(Counting, Quit, true), (Idle, vec![CancelCountdown]));
        assert_eq!(run(Recording, Quit, true), (Idle, vec![StopAndWait]));
        assert_eq!(run(Stopping, Quit, true), (Idle, vec![StopAndWait]));
        for p in [
            Idle,
            Picking(Mode::Screenshot),
            Picking(Mode::Record),
            Starting,
        ] {
            assert_eq!(run(p, Quit, true), (p, vec![]), "{p:?}");
        }
    }

    #[test]
    fn late_events_for_something_already_over_change_nothing() {
        for input in [
            CountdownFinished,
            StartResult { ok: true },
            StartResult { ok: false },
            StreamEnded,
            StopOverdue,
        ] {
            assert_eq!(run(Idle, input, true), (Idle, vec![]), "{input:?}");
        }
        assert_eq!(run(Recording, CountdownFinished, true), (Recording, vec![]));
        assert_eq!(run(Counting, StreamEnded, true), (Counting, vec![]));
    }

    #[test]
    fn a_whole_recording_from_shortcut_to_saved_file() {
        let mut f = Flow::new();
        assert_eq!(f.step(RecordShortcut, true), vec![OpenPicker(Mode::Record)]);
        assert_eq!(
            f.step(PickerClosed { picked: true }, true),
            vec![BeginCountdown]
        );
        assert_eq!(f.step(CountdownFinished, true), vec![StartStream]);
        assert_eq!(f.step(StartResult { ok: true }, true), vec![]);
        assert!(f.running());
        // A screenshot now is refused; the shortcut stops.
        assert_eq!(
            f.step(CaptureRequested(Mode::Screenshot), true),
            vec![ShowBusyNotice]
        );
        assert_eq!(f.step(RecordShortcut, true), vec![StopStream]);
        assert_eq!(f.step(StreamEnded, true), vec![FinishRecording]);
        assert_eq!(f.phase(), Idle);
        assert!(!f.running());
        // And the next one starts fresh.
        assert_eq!(f.step(RecordShortcut, true), vec![OpenPicker(Mode::Record)]);
    }
}
