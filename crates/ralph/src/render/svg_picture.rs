use super::mermaid_picture::installed_families;
use ralph_core::mermaid_fit::first_installed;
use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{self, fontdb};
use std::sync::Arc;

const MONOSPACE: [&str; 4] = [
    "Menlo",
    "DejaVu Sans Mono",
    "Liberation Mono",
    "Courier New",
];

pub fn rasterize_svg(svg: &str) -> Result<Vec<u8>, String> {
    let mut fonts = fontdb::Database::new();
    fonts.load_system_fonts();
    let wanted: Vec<String> = MONOSPACE.iter().map(|name| name.to_string()).collect();
    if let Some(mono) = first_installed(&wanted, &installed_families(&fonts)) {
        fonts.set_monospace_family(mono);
    }
    let options = usvg::Options {
        fontdb: Arc::new(fonts),
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_str(svg, &options).map_err(|error| error.to_string())?;
    let size = tree.size().to_int_size();
    let mut pixmap = Pixmap::new(size.width(), size.height()).ok_or("the chart has no pixels")?;
    resvg::render(&tree, Transform::identity(), &mut pixmap.as_mut());
    pixmap.encode_png().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests;
