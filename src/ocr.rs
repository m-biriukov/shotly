//! On-device text recognition with Apple's Vision framework: the words in a
//! screenshot and where they are. Nothing leaves the Mac.

use std::sync::mpsc;

use anyhow::Result;
use objc2::rc::{Retained, autoreleasepool};
use objc2::{AnyThread, msg_send};
use objc2_core_foundation::{CFRetained, CGRect};
use objc2_core_graphics::CGImage;
use objc2_foundation::{NSArray, NSDictionary, NSError, NSRange};
use objc2_vision::{
    VNImageRequestHandler, VNRecognizeTextRequest, VNRecognizedText, VNRectangleObservation,
    VNRequest, VNRequestTextRecognitionLevel,
};

use crate::capture;
use crate::geom::Rect;

/// A recognized word, where it is (pixels, top-left origin) and which line of
/// the reading order it belongs to.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub text: String,
    pub rect: Rect,
    pub line: usize,
}

/// One line as Vision saw it, before the reading order is decided.
#[derive(Debug)]
struct Line {
    rect: Rect,
    words: Vec<(String, Rect)>,
}

/// Recognizes the words of `image` on a background thread; the result arrives
/// on the returned channel (accurate recognition takes about a second for a
/// full screen).
pub fn start(image: &CFRetained<CGImage>) -> mpsc::Receiver<Result<Vec<Word>, String>> {
    let (tx, rx) = mpsc::channel();
    capture::on_thread(image, move |cg| {
        let result = autoreleasepool(|_| recognize(cg)).map_err(|e| format!("{e:#}"));
        // The picker may be gone by now; then nobody is waiting.
        let _ = tx.send(result);
    });
    rx
}

/// The first recognitions after the app is installed load Vision's models,
/// which takes around a minute; later ones take under a second. Do that once
/// in the background at launch. `screen` should be a real full-screen capture:
/// a small drawn picture leaves part of the models unloaded. The words are
/// thrown away.
pub fn warm_up(screen: &CFRetained<CGImage>) {
    let t = std::time::Instant::now();
    match start(screen).recv() {
        Ok(Ok(_)) => println!("text recognition ready after {:.1?}", t.elapsed()),
        Ok(Err(e)) => eprintln!("text recognition warm-up failed: {e}"),
        Err(_) => eprintln!("text recognition warm-up stopped"),
    }
}

/// `pm` as a `CGImage`.
#[cfg(test)]
fn image_of(pm: &tiny_skia::Pixmap) -> Option<CFRetained<CGImage>> {
    use objc2_core_foundation::CFData;
    use objc2_core_graphics::{
        CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImageAlphaInfo,
    };

    let (width, height) = (pm.width() as usize, pm.height() as usize);
    let data = CFData::from_bytes(pm.data());
    let provider = CGDataProvider::with_cf_data(Some(&data))?;
    let space = CGColorSpace::new_device_rgb()?;
    // SAFETY: the provider holds `width × height` RGBA pixels and `decode`
    // may be null.
    unsafe {
        CGImage::new(
            width,
            height,
            8,
            32,
            width * 4,
            Some(&space),
            CGBitmapInfo(CGImageAlphaInfo::PremultipliedLast.0),
            Some(&provider),
            std::ptr::null(),
            false,
            CGColorRenderingIntent::RenderingIntentDefault,
        )
    }
}

/// For `SHOTLY_TEST_OCR`: how long recognizing each screen takes in this
/// binary.
#[cfg(feature = "dev-hooks")]
pub fn time_screens() -> Result<()> {
    let shots = capture::begin()?.finish()?;
    for pass in 1..=2 {
        for shot in &shots {
            let t = std::time::Instant::now();
            let answer = start(&shot.cg).recv_timeout(std::time::Duration::from_secs(150));
            let words = answer.map(|r| r.map(|w| w.len()));
            println!("pass {pass}: {words:?} words in {:?}", t.elapsed());
        }
    }
    Ok(())
}

fn recognize(cg: &CGImage) -> Result<Vec<Word>> {
    let (width, height) = (CGImage::width(Some(cg)), CGImage::height(Some(cg)));
    let request = VNRecognizeTextRequest::new();
    request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
    request.setUsesLanguageCorrection(true);
    // Latin, Cyrillic and others, whichever the picture holds.
    request.setAutomaticallyDetectsLanguage(true);
    // SAFETY: `cg` outlives the handler (both end with this function), and an
    // empty options dictionary is valid.
    let handler = unsafe {
        VNImageRequestHandler::initWithCGImage_options(
            VNImageRequestHandler::alloc(),
            cg,
            &NSDictionary::new(),
        )
    };
    let as_request: Retained<VNRequest> =
        Retained::into_super(Retained::into_super(request.clone()));
    handler
        .performRequests_error(&NSArray::from_retained_slice(&[as_request]))
        .map_err(|e| anyhow::anyhow!("recognizing text: {}", e.localizedDescription()))?;
    let Some(observations) = request.results() else {
        return Ok(Vec::new());
    };
    let lines = observations
        .iter()
        .filter_map(|observation| {
            let best = observation.topCandidates(1).firstObject()?;
            // SAFETY: a property read on a live observation.
            let bounds = unsafe { observation.boundingBox() };
            let rect = to_pixels(bounds, width, height);
            Some(line_of(&best, rect, width, height))
        })
        .collect();
    Ok(in_reading_order(lines))
}

/// Splits one recognized line into words with a box each.
fn line_of(text: &VNRecognizedText, rect: Rect, width: usize, height: usize) -> Line {
    let string = text.string().to_string();
    let total = string.encode_utf16().count();
    let words = utf16_words(&string)
        .into_iter()
        .map(|(start, len, word)| {
            let found = word_box(text, NSRange::new(start, len))
                // SAFETY: a property read on a live observation.
                .map(|o| to_pixels(unsafe { o.boundingBox() }, width, height))
                .filter(|r| !r.is_empty());
            let rect = found.unwrap_or_else(|| share_of(rect, total, start, len));
            (word.to_string(), rect)
        })
        .collect();
    Line { rect, words }
}

/// Where Vision puts the characters in `range` (UTF-16 units) of `text`.
fn word_box(text: &VNRecognizedText, range: NSRange) -> Option<Retained<VNRectangleObservation>> {
    // SAFETY: `boundingBoxForRange:error:` is declared by VNRecognizedText;
    // it returns an observation or nil with an error, and `range` lies inside
    // the recognized string. (The bindings do not include it.)
    let result: Result<Retained<VNRectangleObservation>, Retained<NSError>> =
        unsafe { msg_send![text, boundingBoxForRange: range, error: _] };
    result.ok()
}

/// The words of `s` as (start, length, word), counted in UTF-16 units as
/// Foundation counts them.
fn utf16_words(s: &str) -> Vec<(usize, usize, &str)> {
    let mut words = Vec::new();
    let mut at = 0;
    // Byte index and UTF-16 index of the word being read.
    let mut open: Option<(usize, usize)> = None;
    for (i, c) in s.char_indices() {
        if c.is_whitespace() {
            if let Some((byte, start)) = open.take() {
                words.push((start, at - start, &s[byte..i]));
            }
        } else if open.is_none() {
            open = Some((i, at));
        }
        at += c.len_utf16();
    }
    if let Some((byte, start)) = open {
        words.push((start, at - start, &s[byte..]));
    }
    words
}

/// Moves `words` from the pixels of an image sized `from` to a window sized
/// `to` (they differ on displays running a scaled resolution).
pub fn scaled(mut words: Vec<Word>, from: (u32, u32), to: (u32, u32)) -> Vec<Word> {
    if from == to || from.0 == 0 || from.1 == 0 {
        return words;
    }
    let sx = f64::from(to.0) / f64::from(from.0);
    let sy = f64::from(to.1) / f64::from(from.1);
    let at = |v: i32, k: f64| (f64::from(v) * k).round() as i32;
    for w in &mut words {
        let r = w.rect;
        w.rect = Rect::new(at(r.x, sx), at(r.y, sy), at(r.w, sx), at(r.h, sy));
    }
    words
}

/// Vision's boxes are fractions of the image with the origin bottom-left.
fn to_pixels(bounds: CGRect, width: usize, height: usize) -> Rect {
    let (w, h) = (width as f64, height as f64);
    let left = bounds.origin.x * w;
    let top = (1.0 - bounds.origin.y - bounds.size.height) * h;
    Rect::new(
        left.round() as i32,
        top.round() as i32,
        (bounds.size.width * w).round() as i32,
        (bounds.size.height * h).round() as i32,
    )
}

/// When Vision gives no box for a word: its share of the line, by length.
fn share_of(line: Rect, total: usize, start: usize, len: usize) -> Rect {
    let total = total.max(1) as f64;
    let x = line.x + (f64::from(line.w) * start as f64 / total).round() as i32;
    let w = (f64::from(line.w) * len as f64 / total).round() as i32;
    Rect::new(x, line.y, w.max(1), line.h)
}

/// Orders lines top to bottom, and those side by side (same height on screen)
/// left to right, then numbers them.
fn in_reading_order(mut lines: Vec<Line>) -> Vec<Word> {
    lines.sort_by_key(|l| l.rect.y + l.rect.h / 2);
    let mut rows: Vec<Vec<Line>> = Vec::new();
    for line in lines {
        let centre = line.rect.y + line.rect.h / 2;
        match rows.last_mut() {
            // On the same row if its centre is within the row's first line.
            Some(row) if centre < row[0].rect.bottom() => row.push(line),
            _ => rows.push(vec![line]),
        }
    }
    rows.into_iter()
        .flat_map(|mut row| {
            row.sort_by_key(|l| l.rect.x);
            row
        })
        .enumerate()
        .flat_map(|(n, line)| {
            line.words.into_iter().map(move |(text, rect)| Word {
                text,
                rect,
                line: n,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_core_foundation::{CGPoint, CGSize};
    use tiny_skia::{Color, Pixmap, Transform};

    #[test]
    fn words_are_counted_in_utf16_units() {
        assert_eq!(
            utf16_words("Hi  there"),
            vec![(0, 2, "Hi"), (4, 5, "there")]
        );
        // Cyrillic letters are one unit each; an emoji is two.
        assert_eq!(
            utf16_words(" Привет 🎉 мир "),
            vec![(1, 6, "Привет"), (8, 2, "🎉"), (11, 3, "мир")]
        );
        assert!(utf16_words("   ").is_empty());
    }

    #[test]
    fn vision_boxes_become_top_left_pixels() {
        // The lower-left quarter of a 200 × 100 image.
        let b = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(0.5, 0.5));
        assert_eq!(to_pixels(b, 200, 100), Rect::new(0, 50, 100, 50));
    }

    #[test]
    fn a_word_without_a_box_gets_its_share_of_the_line() {
        let line = Rect::new(10, 20, 100, 12);
        assert_eq!(share_of(line, 10, 0, 5), Rect::new(10, 20, 50, 12));
        assert_eq!(share_of(line, 10, 6, 4), Rect::new(70, 20, 40, 12));
    }

    #[test]
    fn words_follow_the_window_size() {
        let words = vec![Word {
            text: "a".into(),
            rect: Rect::new(100, 50, 40, 20),
            line: 0,
        }];
        let half = scaled(words.clone(), (200, 100), (100, 50));
        assert_eq!(half[0].rect, Rect::new(50, 25, 20, 10));
        assert_eq!(scaled(words.clone(), (200, 100), (200, 100)), words);
    }

    #[test]
    fn lines_read_top_to_bottom_then_left_to_right() {
        let line = |name: &str, x, y| Line {
            rect: Rect::new(x, y, 50, 10),
            words: vec![(name.to_string(), Rect::new(x, y, 50, 10))],
        };
        let words = in_reading_order(vec![
            line("below", 0, 40),
            line("right", 100, 12),
            line("left", 0, 10),
        ]);
        let read: Vec<(&str, usize)> = words.iter().map(|w| (w.text.as_str(), w.line)).collect();
        assert_eq!(read, vec![("left", 0), ("right", 1), ("below", 2)]);
    }

    /// Recognizes what is on the screens right now, through the same capture
    /// path as the app. Needs Screen Recording permission:
    /// `cargo test live_screen -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs Screen Recording permission"]
    fn live_screen_recognition() {
        let shots = crate::capture::begin().unwrap().finish().unwrap();
        for shot in &shots {
            let size = (
                CGImage::width(Some(&shot.cg)),
                CGImage::height(Some(&shot.cg)),
            );
            let t = std::time::Instant::now();
            let answer = start(&shot.cg).recv_timeout(std::time::Duration::from_secs(180));
            println!(
                "image {size:?}, window points {:?}, took {:?}",
                shot.size_pt,
                t.elapsed()
            );
            let words = answer.expect("answer in time").expect("recognition");
            println!(
                "{} words; first: {:?}",
                words.len(),
                &words[..words.len().min(4)]
            );
        }
    }

    /// Runs the real recognizer on text drawn by the app itself.
    #[test]
    fn real_recognition_reads_drawn_text() {
        let mut pm = Pixmap::new(640, 200).unwrap();
        pm.fill(Color::WHITE);
        let ink = crate::paint::paint(0x000000, 255);
        for (text, y) in [("Hello screenshot", 30.0), ("Second line", 110.0)] {
            crate::text::layout(text, 48.0).draw(
                &mut pm.as_mut(),
                30.0,
                y,
                48.0,
                &ink,
                Transform::identity(),
            );
        }
        let image = image_of(&pm).unwrap();
        let words = start(&image)
            .recv_timeout(std::time::Duration::from_secs(240))
            .unwrap()
            .unwrap();
        let text: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(
            text,
            vec!["Hello", "screenshot", "Second", "line"],
            "{words:?}"
        );
        // Word boxes are separate, in order, and on the right lines.
        assert!(words[0].rect.right() <= words[1].rect.x + 4, "{words:?}");
        assert!(words[2].rect.y > words[0].rect.bottom(), "{words:?}");
        assert_eq!((words[1].line, words[2].line), (0, 1));
    }
}
