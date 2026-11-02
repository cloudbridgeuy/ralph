use super::*;
use crate::config::AppConfig;
use serde_json::json;
use std::fs;
use syntect::highlighting::ThemeSet;
use tempfile::TempDir;

fn theme(name: &str) -> Theme {
    ThemeSet::load_defaults().themes[name].clone()
}

fn ocean() -> Theme {
    theme("base16-ocean.dark")
}

fn style(toml: &str) -> MermaidStyle {
    toml::from_str(toml).unwrap()
}

#[test]
fn defaults_follow_the_code_theme() {
    let (resolved, warnings) = MermaidStyle::default().resolve(&ocean());
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(resolved.colors.bg, [0x2b, 0x30, 0x3b, 255]);
    assert_eq!(resolved.colors.fg, [0xc0, 0xc5, 0xce, 255]);
    assert_eq!(resolved.background, Some(resolved.colors.bg));
    assert_ne!(resolved.colors.accent, resolved.colors.fg);
    assert_ne!(resolved.colors.surface, resolved.colors.bg);
    assert_eq!(
        resolved.font_family,
        ["Arial", "Liberation Sans", "Helvetica Neue", "Helvetica"]
    );
    assert_eq!(
        resolved.font_family_mono,
        ["SF Mono", "Menlo", "DejaVu Sans Mono"]
    );
    assert_eq!(resolved.font_size, 16.0);
    assert_eq!(resolved.padding, 16.0);
    assert!(resolved.auto_direction);
    assert_eq!(resolved.min_text_ratio, 0.6);
    assert!(resolved.config.is_empty());
}

#[test]
fn a_light_code_theme_gives_a_light_picture() {
    let (resolved, _) = MermaidStyle::default().resolve(&theme("InspiredGitHub"));
    assert_eq!(resolved.colors.bg, [0xff, 0xff, 0xff, 255]);
    let [r, g, b, _] = resolved.colors.fg;
    assert!(u16::from(r) + u16::from(g) + u16::from(b) < 300);
}

#[test]
fn a_theme_without_colors_uses_the_dark_palette() {
    let (auto, _) = MermaidStyle::default().resolve(&Theme::default());
    let (dark, _) = style("palette = \"dark\"").resolve(&theme("InspiredGitHub"));
    assert_eq!(auto.colors, dark.colors);
    assert_eq!(dark.colors.bg, [0x2b, 0x30, 0x3b, 255]);
}

#[test]
fn the_light_palette_ignores_the_theme() {
    let (light, _) = style("palette = \"light\"").resolve(&ocean());
    assert_eq!(light.colors.bg, [0xff, 0xff, 0xff, 255]);
    assert_eq!(light.colors.fg, [0x1f, 0x1f, 0x1f, 255]);
    assert_eq!(light.colors.accent, [0x5b, 0x7b, 0xd5, 255]);
}

#[test]
fn each_color_key_overrides_the_palette() {
    let custom = style(
        "series = [\"#000001\", \"#000002\"]\n[colors]\naccent = \"#ff0000\"\nsurface_alt = \"#010203\"\n",
    );
    let (resolved, warnings) = custom.resolve(&ocean());
    assert!(warnings.is_empty(), "{warnings:?}");
    let (base, _) = MermaidStyle::default().resolve(&ocean());
    assert_eq!(resolved.colors.accent, [255, 0, 0, 255]);
    assert_eq!(resolved.colors.surface_alt, [1, 2, 3, 255]);
    assert_eq!(resolved.colors.series[0], [0, 0, 1, 255]);
    assert_eq!(resolved.colors.series[1], [0, 0, 2, 255]);
    assert_eq!(resolved.colors.series[2..], base.colors.series[2..]);
    assert_eq!(resolved.colors.bg, base.colors.bg);
}

#[test]
fn bad_values_warn_and_keep_the_default() {
    let bad = style(
        "palette = \"neon\"\nbackground = \"blue\"\ncurve = \"wiggly\"\nfont_size = 0.0\n\
         padding = -1.0\nmin_text_ratio = 1.5\nfont_family = []\nseries = [\"red\"]\n\
         [colors]\naccent = \"#12\"\nglow = \"#ffffff\"\n",
    );
    let (resolved, warnings) = bad.resolve(&ocean());
    let (base, _) = MermaidStyle::default().resolve(&ocean());
    assert_eq!(resolved, base);
    assert_eq!(
        warnings,
        [
            "[mermaid.style] palette = \"neon\" is not auto, light or dark; using auto",
            "[mermaid.style.colors] accent = \"#12\" is not a #rrggbb color; using the default",
            "[mermaid.style.colors] unknown key \"glow\"; ignored",
            "[mermaid.style] series = \"red\" is not a #rrggbb color; using the default",
            "[mermaid.style] background = \"blue\" is not theme, transparent or #rrggbb; using theme",
            "[mermaid.style] font_family is empty; using the default",
            "[mermaid.style] font_size = 0 is not above 0; using 16",
            "[mermaid.style] padding = -1 is below 0; using 16",
            "[mermaid.style] min_text_ratio = 1.5 is not between 0 and 1; using 0.6",
            "[mermaid.style] curve = \"wiggly\" is not rounded, linear or step; using rounded",
        ]
    );
    assert_eq!(bad.warnings(), warnings);
}

#[test]
fn background_can_be_transparent_or_a_color() {
    let (clear, _) = style("background = \"transparent\"").resolve(&ocean());
    assert_eq!(clear.background, None);
    let (black, _) = style("background = \"#000000\"").resolve(&ocean());
    assert_eq!(black.background, Some([0, 0, 0, 255]));
}

#[test]
fn curve_and_config_pass_through_to_the_site_config() {
    let (linear, _) = style("curve = \"linear\"").resolve(&ocean());
    assert_eq!(
        Value::Object(linear.config),
        json!({"flowchart": {"curve": "linear"}})
    );
    let (rounded, _) = style("curve = \"rounded\"").resolve(&ocean());
    assert!(rounded.config.is_empty());
    let mixed = style(
        "curve = \"step\"\n[config.flowchart]\nnodeSpacing = 30\n[config.sequence]\nmirrorActors = false\n",
    );
    let (resolved, _) = mixed.resolve(&ocean());
    assert_eq!(
        Value::Object(resolved.config),
        json!({
            "flowchart": {"curve": "step", "nodeSpacing": 30},
            "sequence": {"mirrorActors": false}
        })
    );
    let (user_curve, _) =
        style("curve = \"step\"\n[config.flowchart]\ncurve = \"linear\"\n").resolve(&ocean());
    assert_eq!(
        Value::Object(user_curve.config),
        json!({"flowchart": {"curve": "linear"}})
    );
}

#[test]
fn the_store_file_overrides_the_global_style_per_key() {
    let dir = TempDir::new().unwrap();
    let global = dir.path().join("global.toml");
    let store = dir.path().join("store.toml");
    fs::write(
        &global,
        "[mermaid.style]\nfont_size = 14.0\npadding = 8.0\n[mermaid.style.colors]\naccent = \"#111111\"\nbg = \"#222222\"\n",
    )
    .unwrap();
    fs::write(
        &store,
        "[mermaid.style]\nfont_size = 18.0\n[mermaid.style.colors]\naccent = \"#333333\"\n",
    )
    .unwrap();
    let config = AppConfig::load_layers(&global, Some(&store)).unwrap();
    assert_eq!(config.mermaid.picture_timeout_ms, 3000);
    let (resolved, _) = config.mermaid.style.resolve(&ocean());
    assert_eq!(resolved.font_size, 18.0);
    assert_eq!(resolved.padding, 8.0);
    assert_eq!(resolved.colors.accent, [0x33, 0x33, 0x33, 255]);
    assert_eq!(resolved.colors.bg, [0x22, 0x22, 0x22, 255]);
}
