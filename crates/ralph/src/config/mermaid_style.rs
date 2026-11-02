use crate::render::mermaid_picture::{PictureColors, PictureStyle, Rgba};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use syntect::highlighting::{Color, Highlighter, Theme};
use syntect::parsing::Scope;
use toml::Table;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MermaidStyle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub colors: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_family: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_family_mono: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curve: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_direction: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_text_ratio: Option<f64>,
    #[serde(default, skip_serializing_if = "Table::is_empty")]
    pub config: Table,
}

const FONT_FAMILY: [&str; 4] = ["Arial", "Liberation Sans", "Helvetica Neue", "Helvetica"];
const FONT_FAMILY_MONO: [&str; 3] = ["SF Mono", "Menlo", "DejaVu Sans Mono"];
const FONT_SIZE: f64 = 16.0;
const PADDING: f64 = 16.0;
const MIN_TEXT_RATIO: f64 = 0.6;

const LIGHT: PictureColors = PictureColors {
    bg: rgb(0xffffff),
    fg: rgb(0x1f1f1f),
    muted: rgb(0x6e6e6e),
    line: rgb(0x1f1f1f),
    surface: rgb(0xf3f3f3),
    surface_alt: rgb(0xe6e6e6),
    border: rgb(0x8a8a8a),
    accent: rgb(0x5b7bd5),
    critical: rgb(0xe45756),
    series: [
        rgb(0x4c78a8),
        rgb(0xf58518),
        rgb(0x54a24b),
        rgb(0xe45756),
        rgb(0x72b7b2),
        rgb(0xeeca3b),
        rgb(0xb279a2),
        rgb(0x9d755d),
    ],
};

const DARK: PictureColors = PictureColors {
    bg: rgb(0x2b303b),
    fg: rgb(0xc0c5ce),
    muted: rgb(0x65737e),
    line: rgb(0xa7adba),
    surface: rgb(0x343d46),
    surface_alt: rgb(0x4f5b66),
    border: rgb(0x65737e),
    accent: rgb(0x8fa1b3),
    critical: rgb(0xbf616a),
    series: [
        rgb(0x8fa1b3),
        rgb(0xd08770),
        rgb(0xa3be8c),
        rgb(0xbf616a),
        rgb(0x96b5b4),
        rgb(0xebcb8b),
        rgb(0xb48ead),
        rgb(0xab7967),
    ],
};

const SERIES_SCOPES: [&str; 8] = [
    "entity.name.function",
    "string",
    "keyword",
    "constant.numeric",
    "entity.name.tag",
    "support.type",
    "variable.parameter",
    "comment",
];

const CRITICAL_SCOPES: [&str; 2] = ["markup.deleted", "invalid"];

const fn rgb(hex: u32) -> Rgba {
    [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8, 255]
}

fn parse_hex(text: &str) -> Option<Rgba> {
    let digits = text.strip_prefix('#')?;
    if digits.len() != 6 || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(digits, 16).ok().map(rgb)
}

fn opaque(color: Color) -> Rgba {
    [color.r, color.g, color.b, 255]
}

fn mix(over: Rgba, amount: f64, under: Rgba) -> Rgba {
    let channel = |i: usize| {
        (f64::from(over[i]) * amount + f64::from(under[i]) * (1.0 - amount)).round() as u8
    };
    [channel(0), channel(1), channel(2), 255]
}

fn scope_color(highlighter: &Highlighter, scope: &str, fg: Rgba) -> Option<Rgba> {
    let scope = Scope::new(scope).ok()?;
    let color = opaque(highlighter.style_for_stack(&[scope]).foreground);
    (color != fg).then_some(color)
}

fn theme_colors(theme: &Theme) -> PictureColors {
    let (Some(bg), Some(fg)) = (theme.settings.background, theme.settings.foreground) else {
        return DARK;
    };
    let (bg, fg) = (opaque(bg), opaque(fg));
    let highlighter = Highlighter::new(theme);
    let mut picked: Vec<Rgba> = Vec::new();
    for color in SERIES_SCOPES
        .iter()
        .filter_map(|scope| scope_color(&highlighter, scope, fg))
        .chain(DARK.series)
    {
        if !picked.contains(&color) && picked.len() < 8 {
            picked.push(color);
        }
    }
    let mut series = DARK.series;
    series
        .iter_mut()
        .zip(picked)
        .for_each(|(slot, color)| *slot = color);
    let critical = CRITICAL_SCOPES
        .iter()
        .find_map(|scope| scope_color(&highlighter, scope, fg))
        .unwrap_or(DARK.critical);
    PictureColors {
        bg,
        fg,
        muted: mix(fg, 0.55, bg),
        line: mix(fg, 0.75, bg),
        surface: mix(fg, 0.10, bg),
        surface_alt: mix(fg, 0.18, bg),
        border: mix(fg, 0.40, bg),
        accent: series[0],
        critical,
        series,
    }
}

fn color_slot<'a>(colors: &'a mut PictureColors, key: &str) -> Option<&'a mut Rgba> {
    Some(match key {
        "bg" => &mut colors.bg,
        "fg" => &mut colors.fg,
        "muted" => &mut colors.muted,
        "line" => &mut colors.line,
        "surface" => &mut colors.surface,
        "surface_alt" => &mut colors.surface_alt,
        "border" => &mut colors.border,
        "accent" => &mut colors.accent,
        "critical" => &mut colors.critical,
        _ => return None,
    })
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| name.to_string()).collect()
}

impl MermaidStyle {
    pub fn resolve(&self, theme: &Theme) -> (PictureStyle, Vec<String>) {
        let mut warnings = Vec::new();
        let colors = self.colors(theme, &mut warnings);
        let background = match self.background.as_deref() {
            None | Some("theme") => Some(colors.bg),
            Some("transparent") => None,
            Some(other) => parse_hex(other).map(Some).unwrap_or_else(|| {
                warnings.push(format!(
                    "[mermaid.style] background = \"{other}\" is not theme, transparent or #rrggbb; using theme"
                ));
                Some(colors.bg)
            }),
        };
        let font_family = families(
            &self.font_family,
            "font_family",
            &FONT_FAMILY,
            &mut warnings,
        );
        let font_family_mono = families(
            &self.font_family_mono,
            "font_family_mono",
            &FONT_FAMILY_MONO,
            &mut warnings,
        );
        let font_size = checked(self.font_size, FONT_SIZE, &mut warnings, |size| {
            (size <= 0.0 || !size.is_finite())
                .then(|| format!("font_size = {size} is not above 0; using {FONT_SIZE}"))
        });
        let padding = checked(self.padding, PADDING, &mut warnings, |padding| {
            (padding < 0.0 || !padding.is_finite())
                .then(|| format!("padding = {padding} is below 0; using {PADDING}"))
        });
        let min_text_ratio =
            checked(
                self.min_text_ratio,
                MIN_TEXT_RATIO,
                &mut warnings,
                |ratio| {
                    (!(0.0..=1.0).contains(&ratio)).then(|| {
                format!("min_text_ratio = {ratio} is not between 0 and 1; using {MIN_TEXT_RATIO}")
            })
                },
            );
        let config = self.site_config(&mut warnings);
        let style = PictureStyle {
            colors,
            background,
            font_family,
            font_family_mono,
            font_size,
            padding,
            auto_direction: self.auto_direction.unwrap_or(true),
            min_text_ratio,
            config,
        };
        (style, warnings)
    }

    pub fn warnings(&self) -> Vec<String> {
        self.resolve(&Theme::default()).1
    }

    fn colors(&self, theme: &Theme, warnings: &mut Vec<String>) -> PictureColors {
        let mut colors = match self.palette.as_deref() {
            None | Some("auto") => theme_colors(theme),
            Some("light") => LIGHT,
            Some("dark") => DARK,
            Some(other) => {
                warnings.push(format!(
                    "[mermaid.style] palette = \"{other}\" is not auto, light or dark; using auto"
                ));
                theme_colors(theme)
            }
        };
        for (key, value) in &self.colors {
            match (color_slot(&mut colors, key), parse_hex(value)) {
                (Some(slot), Some(color)) => *slot = color,
                (Some(_), None) => warnings.push(format!(
                    "[mermaid.style.colors] {key} = \"{value}\" is not a #rrggbb color; using the default"
                )),
                (None, _) => warnings.push(format!(
                    "[mermaid.style.colors] unknown key \"{key}\"; ignored"
                )),
            }
        }
        let wanted = self.series.iter().flatten();
        for (slot, value) in colors.series.iter_mut().zip(wanted) {
            match parse_hex(value) {
                Some(color) => *slot = color,
                None => warnings.push(format!(
                    "[mermaid.style] series = \"{value}\" is not a #rrggbb color; using the default"
                )),
            }
        }
        colors
    }

    fn site_config(&self, warnings: &mut Vec<String>) -> Map<String, Value> {
        let mut config = Map::new();
        match self.curve.as_deref() {
            None | Some("rounded") => {}
            Some(curve @ ("linear" | "step")) => {
                config.insert(
                    "flowchart".to_string(),
                    Value::Object(Map::from_iter([("curve".to_string(), Value::from(curve))])),
                );
            }
            Some(other) => warnings.push(format!(
                "[mermaid.style] curve = \"{other}\" is not rounded, linear or step; using rounded"
            )),
        }
        match serde_json::to_value(&self.config) {
            Ok(Value::Object(user)) => bezel::config::merge(&mut config, &user),
            Ok(_) | Err(_) => {
                warnings.push("[mermaid.style] config is not a table; ignored".to_string())
            }
        }
        config
    }
}

fn families(
    wanted: &Option<Vec<String>>,
    key: &str,
    default: &[&str],
    warnings: &mut Vec<String>,
) -> Vec<String> {
    match wanted {
        Some(list) if list.is_empty() => {
            warnings.push(format!("[mermaid.style] {key} is empty; using the default"));
            names(default)
        }
        Some(list) => list.clone(),
        None => names(default),
    }
}

fn checked(
    value: Option<f64>,
    default: f64,
    warnings: &mut Vec<String>,
    problem: impl Fn(f64) -> Option<String>,
) -> f64 {
    match value.map(|value| (value, problem(value))) {
        Some((_, Some(warning))) => {
            warnings.push(format!("[mermaid.style] {warning}"));
            default
        }
        Some((value, None)) => value,
        None => default,
    }
}

#[cfg(test)]
mod tests;
