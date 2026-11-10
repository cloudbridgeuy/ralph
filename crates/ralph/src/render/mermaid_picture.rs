use ralph_core::mermaid_fit::{css_families, first_installed, layout_width, picture_fit, Room};
use resvg::tiny_skia::{Color, Pixmap, Transform};
use resvg::usvg::{self, fontdb};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::sync::Arc;

pub type Rgba = [u8; 4];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PictureColors {
    pub bg: Rgba,
    pub fg: Rgba,
    pub muted: Rgba,
    pub line: Rgba,
    pub surface: Rgba,
    pub surface_alt: Rgba,
    pub border: Rgba,
    pub accent: Rgba,
    pub critical: Rgba,
    pub series: [Rgba; 8],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PictureStyle {
    pub colors: PictureColors,
    pub background: Option<Rgba>,
    pub font_family: Vec<String>,
    pub font_family_mono: Vec<String>,
    pub font_size: f64,
    pub padding: f64,
    pub auto_direction: bool,
    pub min_text_ratio: f64,
    pub config: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PictureRequest {
    pub source: String,
    pub style: PictureStyle,
    pub room: Room,
}

pub fn options(style: &PictureStyle, room: &Room) -> bezel::Options {
    let colors = &style.colors;
    bezel::Options {
        font_family: Some(css_families(&style.font_family)),
        font_family_mono: Some(css_families(&style.font_family_mono)),
        font_size: style.font_size,
        responsive: Some(false),
        max_width: Some(layout_width(room, style.font_size)),
        auto_direction: style.auto_direction,
        padding: Some(style.padding),
        links: false,
        images: false,
        config: style.config.clone(),
        colors: Some(bezel::Colors {
            bg: colors.bg,
            fg: colors.fg,
            muted: colors.muted,
            line: colors.line,
            surface: colors.surface,
            surface_alt: colors.surface_alt,
            border: colors.border,
            accent: colors.accent,
            critical: colors.critical,
            series: colors.series,
        }),
        ..bezel::Options::default()
    }
}

pub(super) fn installed_families(fonts: &fontdb::Database) -> Vec<String> {
    fonts
        .faces()
        .flat_map(|face| face.families.iter().map(|(name, _)| name.clone()))
        .collect()
}

pub fn rasterize(request: &PictureRequest, mut fonts: fontdb::Database) -> Result<Vec<u8>, String> {
    let style = &request.style;
    let drawing = bezel::render(
        &request.source,
        &options(style, &request.room),
        &bezel::Metrics,
    )
    .map_err(|error| error.to_string())?;
    let fit = picture_fit(
        (drawing.width, drawing.height),
        style.font_size,
        &request.room,
        style.min_text_ratio,
    )
    .ok_or("the diagram text would be too small to read")?;
    let installed = installed_families(&fonts);
    let sans = first_installed(&style.font_family, &installed).map(str::to_string);
    if let Some(mono) = first_installed(&style.font_family_mono, &installed) {
        fonts.set_monospace_family(mono);
    }
    if let Some(sans) = &sans {
        fonts.set_sans_serif_family(sans);
    }
    let defaults = usvg::Options::default();
    let parse = usvg::Options {
        font_family: sans.unwrap_or(defaults.font_family),
        fontdb: Arc::new(fonts),
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_str(&drawing.svg, &parse).map_err(|error| error.to_string())?;
    let mut pixmap = Pixmap::new(fit.width_px, fit.height_px).ok_or("the picture has no pixels")?;
    if let Some([r, g, b, a]) = style.background {
        pixmap.fill(Color::from_rgba8(r, g, b, a));
    }
    let size = tree.size();
    let transform = Transform::from_scale(
        fit.width_px as f32 / size.width(),
        fit.height_px as f32 / size.height(),
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    pixmap.encode_png().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests;
