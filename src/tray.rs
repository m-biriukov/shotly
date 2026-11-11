//! The two menu-bar icons: the app glyph, and the red dot while recording.

use tray_icon::Icon;

/// Straight-alpha RGBA for the tray (tiny-skia holds premultiplied pixels).
fn to_icon(pm: &tiny_skia::Pixmap) -> Icon {
    let rgba = pm.pixels().iter().flat_map(|p| {
        let c = p.demultiply();
        [c.red(), c.green(), c.blue(), c.alpha()]
    });
    Icon::from_rgba(rgba.collect(), pm.width(), pm.height()).expect("valid icon")
}

/// Side of the tray icons in pixels: 22 pt at 2x.
const TRAY_PX: u32 = 44;

/// Red dot shown instead of the viewfinder while recording (not a template,
/// so macOS keeps its colour).
pub fn recording_icon_image() -> Icon {
    let mut pm = tiny_skia::Pixmap::new(TRAY_PX, TRAY_PX).unwrap();
    if let Some(dot) = tiny_skia::PathBuilder::from_circle(22.0, 22.0, 13.0) {
        pm.fill_path(
            &dot,
            &crate::paint::paint(0xFF3B30, 255),
            tiny_skia::FillRule::Winding,
            tiny_skia::Transform::identity(),
            None,
        );
    }
    to_icon(&pm)
}

/// The menu-bar icon: the app's glyph (viewfinder corners around a camera
/// lens) as a template, black on transparent, so macOS tints it for light and
/// dark menu bars. Made from `assets/menubar-source.png` by
/// `scripts/make_menubar_icon.sh`; shown 22 pt high.
const TRAY_PNG: &[u8] = include_bytes!("../assets/menubar-icon.png");

pub fn tray_icon_image() -> Icon {
    let img = image::load_from_memory(TRAY_PNG)
        .expect("assets/menubar-icon.png is a valid PNG")
        .to_rgba8();
    let (w, h) = img.dimensions();
    Icon::from_rgba(img.into_raw(), w, h).expect("valid icon")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_bar_icon_is_a_black_template_with_a_shape() {
        let img = image::load_from_memory(TRAY_PNG).unwrap().to_rgba8();
        let (w, h) = img.dimensions();
        assert_eq!(h, 88, "shown 22 pt high at 2x");
        assert!(w > h, "wider than tall: {w}x{h}");
        let opaque = img.pixels().filter(|p| p[3] > 200).count();
        let total = (w * h) as usize;
        assert!(
            opaque > total / 8 && opaque < total / 2,
            "a shape, not blank or solid: {opaque}/{total}"
        );
        // Template: the colour is ignored, but keep it black so a preview is true.
        assert!(
            img.pixels()
                .filter(|p| p[3] > 0)
                .all(|p| p.0[..3] == [0, 0, 0])
        );
        // Margin around the glyph.
        assert_eq!(img.get_pixel(0, 0)[3], 0);
    }
}
