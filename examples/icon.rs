//! Draws the app icon and writes `assets/AppIcon.iconset/*.png`.
//! `scripts/make_icon.sh` runs this and packs the result with `iconutil`.

use std::path::Path;

use image::{RgbaImage, imageops::FilterType};
use tiny_skia::{
    Color, FillRule, GradientStop, LineCap, LineJoin, LinearGradient, Paint, PathBuilder, Pixmap,
    Point, SpreadMode, Stroke, Transform,
};

const S: f32 = 1024.0;

fn rounded(x: f32, y: f32, w: f32, h: f32, r: f32) -> tiny_skia::Path {
    // Superellipse-ish corners: cubic with a longer handle than a plain arc.
    let k = r * 0.5523 * 1.12;
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish().unwrap()
}

fn solid(c: Color) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(c);
    p.anti_alias = true;
    p
}

fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color::from_rgba8(r, g, b, a)
}

fn draw() -> Pixmap {
    let mut pm = Pixmap::new(S as u32, S as u32).unwrap();
    let id = Transform::identity();

    // macOS icon grid: 824 px body centred in 1024, soft shadow below it.
    let (x, y, w) = (100.0, 100.0, 824.0);
    for i in 0..14 {
        let grow = i as f32 * 4.0;
        let a = (9 - i / 2).max(1) as u8;
        let body = rounded(
            x - grow,
            y + 14.0 - grow + 18.0,
            w + 2.0 * grow,
            w + 2.0 * grow,
            185.0 + grow,
        );
        pm.fill_path(&body, &solid(rgba(0, 0, 0, a)), FillRule::Winding, id, None);
    }
    let body = rounded(x, y, w, w, 185.0);
    let gradient = LinearGradient::new(
        Point::from_xy(x, y),
        Point::from_xy(x + w, y + w),
        vec![
            GradientStop::new(0.0, rgba(0x4F, 0x9C, 0xFF, 255)),
            GradientStop::new(0.55, rgba(0x4A, 0x5B, 0xF0, 255)),
            GradientStop::new(1.0, rgba(0x7B, 0x3F, 0xE4, 255)),
        ],
        SpreadMode::Pad,
        id,
    )
    .unwrap();
    let paint = Paint {
        shader: gradient,
        anti_alias: true,
        ..Paint::default()
    };
    pm.fill_path(&body, &paint, FillRule::Winding, id, None);

    // Top sheen.
    let sheen = LinearGradient::new(
        Point::from_xy(0.0, y),
        Point::from_xy(0.0, y + w * 0.55),
        vec![
            GradientStop::new(0.0, rgba(255, 255, 255, 70)),
            GradientStop::new(1.0, rgba(255, 255, 255, 0)),
        ],
        SpreadMode::Pad,
        id,
    )
    .unwrap();
    let paint = Paint {
        shader: sheen,
        anti_alias: true,
        ..Paint::default()
    };
    pm.fill_path(&body, &paint, FillRule::Winding, id, None);

    // Selection frame: four white corner brackets.
    let (l, t, r, b) = (250.0, 250.0, 774.0, 774.0);
    let arm = 120.0;
    let mut pb = PathBuilder::new();
    for (cx, cy, dx, dy) in [
        (l, t, 1.0, 1.0),
        (r, t, -1.0, 1.0),
        (l, b, 1.0, -1.0),
        (r, b, -1.0, -1.0),
    ] {
        pb.move_to(cx, cy + dy * arm);
        pb.line_to(cx, cy);
        pb.line_to(cx + dx * arm, cy);
    }
    let stroke = Stroke {
        width: 46.0,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    pm.stroke_path(
        &pb.finish().unwrap(),
        &solid(rgba(255, 255, 255, 245)),
        &stroke,
        id,
        None,
    );

    // Annotation arrow across the frame, in the app's default red.
    let (ax, ay, bx, by): (f32, f32, f32, f32) = (352.0, 662.0, 648.0, 366.0);
    let (dx, dy) = (bx - ax, by - ay);
    let len = (dx * dx + dy * dy).sqrt();
    let (ux, uy) = (dx / len, dy / len);
    let head = 150.0;
    let shaft_end = (bx - ux * head * 0.7, by - uy * head * 0.7);
    let mut pb = PathBuilder::new();
    pb.move_to(ax, ay);
    pb.line_to(shaft_end.0, shaft_end.1);
    let red = solid(rgba(0xFF, 0x4D, 0x42, 255));
    let shaft = Stroke {
        width: 62.0,
        line_cap: LineCap::Round,
        ..Stroke::default()
    };
    // White outline first so the arrow reads on any part of the gradient.
    let outline = Stroke {
        width: 62.0 + 26.0,
        line_cap: LineCap::Round,
        ..Stroke::default()
    };
    let path = pb.finish().unwrap();
    pm.stroke_path(&path, &solid(rgba(255, 255, 255, 255)), &outline, id, None);
    let (bxm, bym) = (bx - ux * head, by - uy * head);
    let half = head * 0.62;
    let mut tri = PathBuilder::new();
    tri.move_to(bx + ux * 18.0, by + uy * 18.0);
    tri.line_to(bxm - uy * half, bym + ux * half);
    tri.line_to(bxm + uy * half, bym - ux * half);
    tri.close();
    let tri = tri.finish().unwrap();
    let join = Stroke {
        width: 26.0,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    pm.stroke_path(&tri, &solid(rgba(255, 255, 255, 255)), &join, id, None);
    pm.stroke_path(&path, &red, &shaft, id, None);
    pm.fill_path(&tri, &red, FillRule::Winding, id, None);
    pm.stroke_path(
        &tri,
        &red,
        &Stroke {
            width: 10.0,
            line_join: LineJoin::Round,
            ..Stroke::default()
        },
        id,
        None,
    );

    pm
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "assets/AppIcon.iconset".into());
    let out = Path::new(&out);
    std::fs::create_dir_all(out).unwrap();
    let pm = draw();
    // Premultiplied → straight alpha for the encoder.
    let mut straight = Vec::with_capacity(pm.data().len());
    for p in pm.pixels() {
        let c = p.demultiply();
        straight.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    let master = RgbaImage::from_raw(S as u32, S as u32, straight).unwrap();
    for (name, size) in [
        ("icon_16x16", 16),
        ("icon_16x16@2x", 32),
        ("icon_32x32", 32),
        ("icon_32x32@2x", 64),
        ("icon_128x128", 128),
        ("icon_128x128@2x", 256),
        ("icon_256x256", 256),
        ("icon_256x256@2x", 512),
        ("icon_512x512", 512),
        ("icon_512x512@2x", 1024),
    ] {
        let img = image::imageops::resize(&master, size, size, FilterType::Lanczos3);
        img.save(out.join(format!("{name}.png"))).unwrap();
    }
    println!("wrote {}", out.display());
}
