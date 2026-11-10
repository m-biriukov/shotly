//! Developer-only profiling of a live session (`--features dev-hooks`).

use std::time::Instant;

use super::{Session, Win};
use crate::capture::Shot;
use crate::editor::Editor;
use crate::geom::Point;
use crate::render::Backdrop;
use crate::style::Tool;

/// `SCREENSHOOT_PROFILE=1`: times the heaviest interactions. It draws into
/// the live capture, so it only exists in `--features dev-hooks` builds.
pub(super) fn run(session: &mut Session) {
    let Some(w) = session.wins.first_mut() else {
        return;
    };
    let (bw, bh) = w.size();
    let (bw, bh) = (bw as i32, bh as i32);
    let time = |w: &mut Win, shots: &[Shot], f: &mut dyn FnMut(&mut Editor, i32)| {
        let t = Instant::now();
        for k in 0..120 {
            f(&mut w.editor, k);
            if w.backdrop.is_none() {
                let (a, b) = w.size();
                w.backdrop = Some(Backdrop::new(shots[w.shot].image(), a, b));
            }
            w.layers.sync(&w.editor, w.backdrop.as_ref(), false);
        }
        t.elapsed() / 120
    };
    w.editor.press(Point::new(0, 0), false);
    w.editor.release(Point::new(bw, bh), false);
    w.editor.set_tool(Tool::Pen);
    w.editor.press(Point::new(100, 100), false);
    let pen = time(w, &session.shots, &mut |e, k| {
        e.motion(Point::new(100 + k * 20, 300 + (k % 30) * 40), false);
    });
    w.editor.release(Point::new(2500, 900), false);
    w.editor.set_tool(Tool::Select);
    w.editor.press(Point::new(bw / 2, bh / 2), false);
    w.editor.release(Point::new(bw / 2, bh / 2), false);
    // Shrink the area, then drag it around: the common "adjust selection" case.
    w.editor.press(Point::new(bw, bh), false);
    w.editor.release(Point::new(bw * 2 / 3, bh * 2 / 3), false);
    w.editor.press(Point::new(bw / 3, bh / 3), false);
    let mv = time(w, &session.shots, &mut |e, k| {
        e.motion(Point::new(bw / 3 + k, bh / 3 + k / 2), false);
    });
    w.editor.release(Point::new(bw / 3, bh / 3), false);
    let hover = time(w, &session.shots, &mut |e, k| {
        e.motion(Point::new(k * 10, bh - 5), false);
    });
    eprintln!(
        "profile: {bw}x{bh} per update: drawing pen {pen:?}, moving area {mv:?}, hover {hover:?}"
    );
}
