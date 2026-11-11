//! Turning a selection into a finished image: crop, copy, save.

use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use image::{RgbaImage, imageops};

use crate::config::Format;
use crate::geom::Rect;
use crate::paint;
use crate::shape::Shape;

/// Crops `img` to `sel` and paints `shapes` on top. `sel` and the shapes are
/// in a window of `win_w`×`win_h` pixels showing the whole image (the two may
/// differ in resolution); the result is at full image resolution.
pub fn export(img: &RgbaImage, sel: Rect, shapes: &[Shape], win_w: u32, win_h: u32) -> RgbaImage {
    let (iw, ih) = img.dimensions();
    let map = |v: i32, from: u32, to: u32| (v.max(0) as u64 * to as u64 / from as u64) as u32;
    let x0 = map(sel.x, win_w, iw).min(iw);
    let y0 = map(sel.y, win_h, ih).min(ih);
    let x1 = map(sel.right(), win_w, iw).min(iw);
    let y1 = map(sel.bottom(), win_h, ih).min(ih);
    let cropped = imageops::crop_imm(img, x0, y0, x1 - x0, y1 - y0).to_image();
    let (w, h) = cropped.dimensions();
    if shapes.is_empty() || w == 0 || h == 0 {
        return cropped;
    }
    // The screenshot is opaque, so its RGBA bytes are already premultiplied.
    let size = tiny_skia::IntSize::from_wh(w, h).unwrap();
    let mut pm = tiny_skia::Pixmap::from_vec(cropped.into_raw(), size).unwrap();
    let ts = tiny_skia::Transform::from_scale(iw as f32 / win_w as f32, ih as f32 / win_h as f32)
        .post_translate(-(x0 as f32), -(y0 as f32));
    for s in shapes {
        paint::draw_shape(&mut pm.as_mut(), s, ts);
    }
    RgbaImage::from_raw(w, h, pm.take()).unwrap()
}

pub fn copy_to_clipboard(img: &RgbaImage) -> Result<()> {
    let mut cb = arboard::Clipboard::new().context("opening clipboard")?;
    cb.set_image(arboard::ImageData {
        width: img.width() as usize,
        height: img.height() as usize,
        bytes: img.as_raw().into(),
    })
    .context("writing image to clipboard")
}

/// Saves `img` into `dir` as `name` (which includes the extension), adding
/// " (2)", " (3)"… instead of overwriting. JPEG uses quality 90.
pub fn save(img: &RgbaImage, dir: &Path, name: &str, format: Format) -> Result<PathBuf> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, format.ext()));
    // Claim the name atomically (create_new), so two saves in the same
    // second can never overwrite each other or an existing file.
    let (path, file) = (1..10_000)
        .map(|n| {
            if n == 1 {
                dir.join(name)
            } else {
                dir.join(format!("{stem} ({n}).{ext}"))
            }
        })
        .find_map(|p| {
            // Owner-only (0600): screenshots can hold sensitive content, and the
            // save folder might be shared with other users of this Mac.
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&p)
            {
                Ok(f) => Some(Ok((p, f))),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(e) => Some(Err(e)),
            }
        })
        .context("no free file name")?
        .with_context(|| format!("creating a file in {}", dir.display()))?;
    let mut out = std::io::BufWriter::new(file);
    let written = match format {
        Format::Png => img
            .write_to(&mut out, image::ImageFormat::Png)
            .map_err(anyhow::Error::from),
        Format::Jpeg => {
            let rgb = image::DynamicImage::ImageRgba8(img.clone()).to_rgb8();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90)
                .encode_image(&rgb)
                .map_err(anyhow::Error::from)
        }
    }
    .and_then(|()| std::io::Write::flush(&mut out).map_err(anyhow::Error::from));
    if let Err(e) = written {
        // Don't leave a truncated image behind.
        drop(out);
        let _ = std::fs::remove_file(&path);
        return Err(e.context(format!("writing {}", path.display())));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn export_maps_window_pixels_to_image_pixels() {
        let mut img = RgbaImage::new(200, 100);
        img.put_pixel(40, 20, Rgba([1, 2, 3, 255]));
        let out = export(&img, Rect::new(20, 10, 30, 15), &[], 100, 50);
        assert_eq!(out.dimensions(), (60, 30));
        assert_eq!(out.get_pixel(0, 0), &Rgba([1, 2, 3, 255]));
    }

    #[test]
    fn export_is_clamped_to_image() {
        let img = RgbaImage::new(100, 100);
        let out = export(&img, Rect::new(90, 90, 50, 50), &[], 100, 100);
        assert_eq!(out.dimensions(), (10, 10));
    }

    #[test]
    fn shapes_are_painted_at_image_resolution() {
        use crate::geom::Point;
        use crate::shape::{Kind, Style};
        let img = RgbaImage::from_pixel(200, 200, Rgba([255, 255, 255, 255]));
        let mut line = Shape::new(
            Kind::Line,
            Point::new(20, 30),
            Style {
                color: 0x0000FF,
                size: 2.0,
                bg: false,
            },
        );
        line.drag_to(Point::new(60, 30), false);
        // Window is half the image resolution; area starts at (10, 10).
        let out = export(&img, Rect::new(10, 10, 80, 80), &[line], 100, 100);
        assert_eq!(out.dimensions(), (160, 160));
        assert_eq!(out.get_pixel(40, 40), &Rgba([0, 0, 255, 255]));
        assert_eq!(out.get_pixel(40, 60), &Rgba([255, 255, 255, 255]));
    }

    #[test]
    fn save_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("shotly-test-{}", std::process::id()));
        let img = RgbaImage::new(4, 4);
        let a = save(&img, &dir, "shot.png", Format::Png).unwrap();
        let b = save(&img, &dir, "shot.png", Format::Png).unwrap();
        assert_ne!(a, b);
        assert_eq!(b.file_name().unwrap(), "shot (2).png");
        let j = save(&img, &dir, "shot.jpg", Format::Jpeg).unwrap();
        for p in [&a, &b, &j] {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{} is readable by others", p.display());
        }
        assert_eq!(
            &std::fs::read(&j).unwrap()[..2],
            &[0xFF, 0xD8],
            "JPEG magic"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
