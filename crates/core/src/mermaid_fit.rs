use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Room {
    pub cell_px: (u32, u32),
    pub cols: u32,
    pub rows: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    pub width_px: u32,
    pub height_px: u32,
    pub scale: f64,
}

pub fn layout_width(room: &Room, font_size: f64) -> f64 {
    let (cell_w, cell_h) = room.cell_px;
    f64::from(room.cols) * f64::from(cell_w) * font_size / f64::from(cell_h)
}

pub fn picture_fit(size: (f64, f64), font_size: f64, room: &Room, min_ratio: f64) -> Option<Fit> {
    let (width, height) = size;
    let (cell_w, cell_h) = (f64::from(room.cell_px.0), f64::from(room.cell_px.1));
    if !(width > 0.0 && height > 0.0 && font_size > 0.0 && cell_h > 0.0) {
        return None;
    }
    let scale = (f64::from(room.cols) * cell_w / width)
        .min(f64::from(room.rows) * cell_h / height)
        .min(cell_h / font_size);
    if !(scale.is_finite() && scale > 0.0) || font_size * scale < min_ratio * cell_h {
        return None;
    }
    let pixels = |length: f64| (length * scale).round().max(1.0) as u32;
    Some(Fit {
        width_px: pixels(width),
        height_px: pixels(height),
        scale,
    })
}

pub fn first_installed<'a>(wanted: &'a [String], installed: &[String]) -> Option<&'a str> {
    wanted
        .iter()
        .find(|name| installed.iter().any(|have| have.eq_ignore_ascii_case(name)))
        .map(String::as_str)
}

pub fn css_families(names: &[String]) -> String {
    names
        .iter()
        .map(|name| {
            if name.contains(' ') {
                format!("'{name}'")
            } else {
                name.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests;
