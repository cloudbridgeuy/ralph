use super::*;
use crate::config::{MermaidStyle, MermaidStyleExt};
use resvg::tiny_skia::Pixmap;
use serde_json::json;
use syntect::highlighting::ThemeSet;

const ROOM: Room = Room {
    cell_px: (10, 20),
    cols: 80,
    rows: 40,
};

fn style() -> PictureStyle {
    let themes = ThemeSet::load_defaults();
    MermaidStyle::default()
        .resolve(
            &themes.themes["base16-ocean.dark"],
            &crate::ui_theme::UiColors::default(),
        )
        .0
}

fn request(source: &str, style: PictureStyle) -> PictureRequest {
    PictureRequest {
        source: source.to_string(),
        style,
        room: ROOM,
    }
}

#[test]
fn options_carry_the_style_and_the_room() {
    let mut picked = style();
    picked.config = json!({"flowchart": {"curve": "linear"}})
        .as_object()
        .cloned()
        .unwrap();
    let built = options(&picked, &ROOM);
    let colors = built.colors.unwrap();
    assert_eq!(colors.bg, picked.colors.bg);
    assert_eq!(colors.series, picked.colors.series);
    assert_eq!(
        built.font_family.as_deref(),
        Some("Arial, 'Liberation Sans', 'Helvetica Neue', Helvetica")
    );
    assert_eq!(
        built.font_family_mono.as_deref(),
        Some("'SF Mono', Menlo, 'DejaVu Sans Mono'")
    );
    assert_eq!(built.font_size, 16.0);
    assert_eq!(built.padding, Some(24.0));
    assert_eq!(built.max_width, Some(640.0));
    assert_eq!(built.responsive, Some(false));
    assert!(built.auto_direction);
    assert!(!built.links);
    assert!(!built.images);
    assert_eq!(built.config, picked.config);
}

#[test]
fn a_small_flowchart_rasterizes_to_a_png_inside_the_room() {
    let png = rasterize(
        &request("flowchart LR\n  A[Start] --> B[End]\n", style()),
        fontdb::Database::new(),
    )
    .unwrap();
    let pixmap = Pixmap::decode_png(&png).unwrap();
    assert!(pixmap.width() > 0 && pixmap.width() <= 800);
    assert!(pixmap.height() > 0 && pixmap.height() <= 800);
    let corner = pixmap.pixel(0, 0).unwrap();
    assert_eq!(
        [corner.red(), corner.green(), corner.blue(), corner.alpha()],
        [0x1e, 0x21, 0x27, 255]
    );
}

#[test]
fn a_transparent_background_leaves_the_corner_clear() {
    let mut clear = style();
    clear.background = None;
    let png = rasterize(
        &request("flowchart LR\n  A --> B\n", clear),
        fontdb::Database::new(),
    )
    .unwrap();
    let pixmap = Pixmap::decode_png(&png).unwrap();
    assert_eq!(pixmap.pixel(0, 0).unwrap().alpha(), 0);
}

#[test]
fn a_diagram_too_big_for_readable_text_is_refused() {
    let chain: String = (0..200)
        .map(|i| format!("  N{i} --> N{}\n", i + 1))
        .collect();
    let refused = rasterize(
        &request(&format!("flowchart TD\n{chain}"), style()),
        fontdb::Database::new(),
    );
    assert!(refused.is_err());
}

#[test]
fn a_source_that_is_not_a_diagram_is_refused() {
    let refused = rasterize(&request("hello there", style()), fontdb::Database::new());
    assert!(refused.is_err());
}
