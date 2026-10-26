use std::io::{IsTerminal, Read, Write};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Instant;

use ralph_core::kitty::{
    cell_grid, encode_apc, graphics_reply, parse_image_markers, png_size, should_query_graphics,
    strip_kitty_apc, GraphicsReply, Segment, TerminalSnapshot, GRAPHICS_QUERY,
    GRAPHICS_QUERY_TIMEOUT,
};

use super::text::render_markdown;
use super::RenderContext;
use crate::markdown::MarkdownTheme;

const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageSettings {
    pub cell_px: Option<(u32, u32)>,
}

static IMAGES: OnceLock<Option<ImageSettings>> = OnceLock::new();

pub fn enable_images(settings: Option<ImageSettings>) {
    let _ = IMAGES.set(settings);
}

pub fn enabled() -> Option<ImageSettings> {
    IMAGES.get().copied().flatten()
}

struct RawMode;

impl RawMode {
    fn enter() -> Option<Self> {
        crossterm::terminal::enable_raw_mode().ok().map(|()| Self)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

fn read_ready(timeout_ms: libc::c_int, into: &mut Vec<u8>) -> bool {
    let mut input = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    if unsafe { libc::poll(&mut input, 1, timeout_ms) } <= 0 || input.revents & libc::POLLIN == 0 {
        return false;
    }
    let mut buffer = [0u8; 256];
    let count = unsafe { libc::read(libc::STDIN_FILENO, buffer.as_mut_ptr().cast(), buffer.len()) };
    match usize::try_from(count) {
        Ok(count) if count > 0 => {
            into.extend_from_slice(&buffer[..count]);
            true
        }
        _ => false,
    }
}

fn query_graphics() -> Option<GraphicsReply> {
    let _raw = RawMode::enter()?;
    let mut stdout = std::io::stdout();
    stdout.write_all(GRAPHICS_QUERY.as_bytes()).ok()?;
    stdout.flush().ok()?;
    let started = Instant::now();
    let mut bytes = Vec::new();
    loop {
        match graphics_reply(&bytes) {
            GraphicsReply::Pending => {}
            decided => return Some(decided),
        }
        let remaining = GRAPHICS_QUERY_TIMEOUT.checked_sub(started.elapsed())?;
        let wait = libc::c_int::try_from(remaining.as_millis().max(1)).ok()?;
        if !read_ready(wait, &mut bytes) {
            return None;
        }
    }
}

pub fn detect() -> Option<ImageSettings> {
    let snapshot = TerminalSnapshot {
        stdin_tty: std::io::stdin().is_terminal(),
        stdout_tty: std::io::stdout().is_terminal(),
        tmux: std::env::var_os("TMUX").is_some(),
    };
    if !should_query_graphics(&snapshot) {
        return None;
    }
    (query_graphics()? == GraphicsReply::Supported).then(|| ImageSettings {
        cell_px: crossterm::terminal::window_size()
            .ok()
            .and_then(|size| {
                let width = u32::from(size.width).checked_div(u32::from(size.columns))?;
                let height = u32::from(size.height).checked_div(u32::from(size.rows))?;
                Some((width, height))
            })
            .filter(|(width, height)| *width > 0 && *height > 0),
    })
}

fn resolve(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(path),
    }
}

fn read_capped(path: &str) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(resolve(path))
        .ok()?
        .take(MAX_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= MAX_IMAGE_BYTES).then_some(bytes)
}

fn fallback(alt: &str, path: &str) -> String {
    format!("[image: {alt} ({path})]\n")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageBounds {
    pub cols: u32,
    pub rows: u32,
}

fn render_image(alt: &str, path: &str, images: ImageSettings, bounds: ImageBounds) -> String {
    let Some(bytes) = read_capped(path) else {
        return fallback(alt, path);
    };
    let Some(size) = png_size(&bytes) else {
        return fallback(alt, path);
    };
    let (cols, rows) = cell_grid(size, images.cell_px, bounds.cols, bounds.rows);
    let newlines = (0..rows).map(|_| '\n').collect::<String>();
    format!("{}{newlines}", encode_apc(&bytes, cols, rows))
}

pub fn render_with_markers(
    content: &str,
    images: ImageSettings,
    bounds: ImageBounds,
    render_text: impl Fn(&str) -> String,
) -> String {
    let mut out = String::new();
    let mut after_image = false;
    for segment in parse_image_markers(content) {
        match segment {
            Segment::Text(text) => {
                let text = if after_image {
                    text.strip_prefix('\n').unwrap_or(&text)
                } else {
                    &text
                };
                after_image = false;
                out.push_str(&render_text(text));
            }
            Segment::Image { alt, path } => {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str(&render_image(&alt, &path, images, bounds));
                after_image = true;
            }
        }
    }
    out
}

pub fn render_cell_display(
    ctx: &RenderContext<'_>,
    text: &str,
    markdown: &MarkdownTheme,
) -> String {
    match ctx.images {
        Some(_) => render_markdown(ctx, text, markdown),
        None => render_markdown(ctx, &strip_kitty_apc(text), markdown),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SETTINGS: ImageSettings = ImageSettings { cell_px: None };
    const BOUNDS: ImageBounds = ImageBounds { cols: 80, rows: 50 };

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ralph-image-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn unreadable_path_falls_back_to_text() {
        assert_eq!(
            render_image("chart", "/definitely/not/here.png", SETTINGS, BOUNDS),
            "[image: chart (/definitely/not/here.png)]\n"
        );
    }

    #[test]
    fn non_png_file_falls_back_to_text() {
        let dir = temp_dir("non-png");
        let path = dir.join("a.png");
        std::fs::write(&path, b"not a png at all, just text bytes").unwrap();
        let shown = path.to_string_lossy().into_owned();
        assert_eq!(
            render_image("a", &shown, SETTINGS, BOUNDS),
            format!("[image: a ({shown})]\n")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn markers_render_text_around_the_fallback() {
        let out = render_with_markers("a ![x](/no/such.png) b", SETTINGS, BOUNDS, |t| {
            t.to_string()
        });
        assert_eq!(out, "a \n[image: x (/no/such.png)]\n b");
    }

    #[test]
    fn fallback_as_last_segment_ends_with_a_line_end() {
        let out = render_with_markers("![x](/no.png)", SETTINGS, BOUNDS, |t| t.to_string());
        assert_eq!(out, "[image: x (/no.png)]\n");
    }

    #[test]
    fn marker_line_end_is_not_doubled() {
        let out = render_with_markers("![x](/no.png)\nnext", SETTINGS, BOUNDS, |t| t.to_string());
        assert_eq!(out, "[image: x (/no.png)]\nnext");
    }

    #[test]
    fn valid_png_renders_kitty_apc_and_row_newlines() {
        let dir = temp_dir("valid");
        let path = dir.join("tall.png");
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&16u32.to_be_bytes());
        png.extend_from_slice(&64u32.to_be_bytes());
        std::fs::write(&path, &png).unwrap();
        let out = render_image("t", &path.to_string_lossy(), SETTINGS, BOUNDS);
        std::fs::remove_dir_all(&dir).unwrap();
        let (cols, rows) = cell_grid((16, 64), None, BOUNDS.cols, BOUNDS.rows);
        assert!(out.starts_with("\x1b_G"));
        for key in ["f=100", "C=1", &format!("c={cols}"), &format!("r={rows}")] {
            assert!(out.contains(key), "{key} missing in {out:?}");
        }
        let body = out.trim_end_matches('\n');
        assert_eq!(out.len() - body.len(), rows as usize);
        assert!(body.ends_with("\x1b\\"));
    }
}
