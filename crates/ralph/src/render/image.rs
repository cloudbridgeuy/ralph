use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use ralph_core::attachment::attachment_paths;
use ralph_core::kitty::{
    cell_grid, clamp_placeholder_grid, encode_apc, encode_virtual, graphics_probe, graphics_query,
    graphics_reply, parse_image_markers, placeholder_rows, strip_kitty_apc, GraphicsReply,
    Placement, Segment, TerminalSnapshot, GRAPHICS_QUERY_TIMEOUT,
};

use ralph_core::kitty_image::prepare;

use super::text::render_markdown;
use super::wrap::wrap_width;
use super::RenderContext;
use crate::ansi::{DIM, RESET};
use crate::chat::terminal::{poll_input, read_input, Input, PartialMode};
use crate::markdown::MarkdownTheme;
use termimad::terminal_size;

const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;
const TMUX_PROBE_TIMEOUT: Duration = Duration::from_millis(300);
const TMUX_PROBE_POLL: Duration = Duration::from_millis(10);

static NEXT_IMAGE_ID: AtomicU8 = AtomicU8::new(1);

fn next_image_id() -> u8 {
    NEXT_IMAGE_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| {
            Some(if id == u8::MAX { 1 } else { id + 1 })
        })
        .unwrap_or(1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageSettings {
    pub placement: Placement,
    pub cell_px: Option<(u32, u32)>,
}

static IMAGES: OnceLock<Option<ImageSettings>> = OnceLock::new();

pub fn enable_images(settings: Option<ImageSettings>) {
    let _ = IMAGES.set(settings);
}

pub fn enabled() -> Option<ImageSettings> {
    IMAGES.get().copied().flatten()
}

fn query_graphics(placement: Placement) -> Option<GraphicsReply> {
    let _raw = PartialMode::enter()?;
    let mut stdout = std::io::stdout();
    stdout
        .write_all(graphics_query(placement).as_bytes())
        .ok()?;
    stdout.flush().ok()?;
    let started = Instant::now();
    let mut bytes = Vec::new();
    loop {
        match graphics_reply(&bytes, placement) {
            GraphicsReply::Pending => {}
            decided => return Some(decided),
        }
        let remaining = GRAPHICS_QUERY_TIMEOUT.checked_sub(started.elapsed())?;
        let wait = libc::c_int::try_from(remaining.as_millis().max(1)).ok()?;
        match poll_input(wait) {
            Input::Ready => {
                let read = read_input();
                if read.is_empty() {
                    return None;
                }
                bytes.extend(read);
            }
            Input::Idle | Input::Closed => return None,
        }
    }
}

fn tmux_passthrough() -> Option<String> {
    let pane = std::env::var("TMUX_PANE").ok()?;
    let mut child = Command::new("tmux")
        .args(["display", "-p", "-t", &pane, "#{allow-passthrough}"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < TMUX_PROBE_TIMEOUT => {
                std::thread::sleep(TMUX_PROBE_POLL);
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    status.success().then(|| out.trim().to_string())
}

pub fn detect(color: bool) -> Option<ImageSettings> {
    let stdin_tty = std::io::stdin().is_terminal();
    let stdout_tty = std::io::stdout().is_terminal();
    let tmux = std::env::var_os("TMUX").is_some();
    let snapshot = TerminalSnapshot {
        stdin_tty,
        stdout_tty,
        color,
        tmux,
        passthrough: (stdin_tty && stdout_tty && tmux)
            .then(tmux_passthrough)
            .flatten(),
    };
    let placement = graphics_probe(&snapshot)?;
    (query_graphics(placement)? == GraphicsReply::Supported).then(|| ImageSettings {
        placement,
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

fn read_capped(path: &Path) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= MAX_IMAGE_BYTES).then_some(bytes)
}

fn file_url(path: &Path) -> String {
    let mut url = String::from("file://");
    for &byte in path.as_os_str().as_encoded_bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
            url.push(char::from(byte));
        } else {
            url.push_str(&format!("%{byte:02X}"));
        }
    }
    url
}

fn path_line(path: &Path) -> String {
    format!(
        "{DIM}\x1b]8;;{}\x1b\\{}\x1b]8;;\x1b\\{RESET}\n",
        file_url(path),
        path.display()
    )
}

fn fallback(alt: &str, path: &str) -> String {
    format!("[image: {alt} ({path})]\n")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageBounds {
    pub cols: u32,
    pub rows: u32,
}

impl ImageBounds {
    pub fn current() -> Self {
        let (cols, rows) = terminal_size();
        Self {
            cols: wrap_width(cols.into()).try_into().unwrap_or(u32::MAX),
            rows: u32::from(rows.max(1)),
        }
    }
}

fn render_image(alt: &str, path: &str, images: ImageSettings, bounds: ImageBounds) -> String {
    draw_image(path, images, bounds).unwrap_or_else(|| fallback(alt, path))
}

pub fn render_attachments(text: &str, images: ImageSettings, bounds: ImageBounds) -> String {
    attachment_paths(text)
        .iter()
        .filter_map(|path| draw_image(path, images, bounds))
        .collect()
}

fn draw_image(path: &str, images: ImageSettings, bounds: ImageBounds) -> Option<String> {
    let resolved = resolve(path);
    let bytes = read_capped(&resolved)?;
    let image = prepare(&bytes)?;
    let grid = cell_grid(image.size, images.cell_px, bounds.cols, bounds.rows);
    let drawn = match images.placement {
        Placement::Direct => {
            let (cols, rows) = grid;
            let newlines = (0..rows).map(|_| '\n').collect::<String>();
            format!("{}{newlines}", encode_apc(&image, cols, rows))
        }
        Placement::Tmux => {
            let (cols, rows) = clamp_placeholder_grid(grid);
            let id = next_image_id();
            format!(
                "{}{}",
                encode_virtual(&image, id, cols, rows),
                placeholder_rows(id, cols, rows)
            )
        }
    };
    let shown = std::fs::canonicalize(&resolved)
        .or_else(|_| std::path::absolute(&resolved))
        .unwrap_or(resolved);
    Some(drawn + &path_line(&shown))
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
        Some(images) if images.placement == Placement::Direct => {
            render_markdown(ctx, text, markdown)
        }
        _ => render_markdown(ctx, &strip_kitty_apc(text), markdown),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SETTINGS: ImageSettings = ImageSettings {
        placement: Placement::Direct,
        cell_px: None,
    };
    const BOUNDS: ImageBounds = ImageBounds { cols: 80, rows: 50 };

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ralph-image-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn file_url_encodes_space_hash_question_percent_and_non_ascii() {
        assert_eq!(
            file_url(Path::new("/tmp/a b#c?d%é.png")),
            "file:///tmp/a%20b%23c%3Fd%25%C3%A9.png"
        );
    }

    #[test]
    fn file_url_keeps_slashes_and_unreserved_chars() {
        assert_eq!(
            file_url(Path::new("/Users/me/A-z_0.9~/x.png")),
            "file:///Users/me/A-z_0.9~/x.png"
        );
    }

    #[test]
    fn path_line_wraps_plain_path_in_dim_osc8_link() {
        assert_eq!(
            path_line(Path::new("/tmp/a b.png")),
            "\x1b[2m\x1b]8;;file:///tmp/a%20b.png\x1b\\/tmp/a b.png\x1b]8;;\x1b\\\x1b[0m\n"
        );
    }

    #[test]
    fn unreadable_path_falls_back_to_text() {
        assert_eq!(
            render_image("chart", "/definitely/not/here.png", SETTINGS, BOUNDS),
            "[image: chart (/definitely/not/here.png)]\n"
        );
    }

    fn tall_png(dir: &std::path::Path) -> PathBuf {
        let path = dir.join("tall.png");
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&16u32.to_be_bytes());
        png.extend_from_slice(&64u32.to_be_bytes());
        std::fs::write(&path, &png).unwrap();
        path
    }

    #[test]
    fn attachments_draw_only_drawable_pngs() {
        let dir = temp_dir("attach");
        let good = tall_png(&dir);
        let bad = dir.join("bad.png");
        std::fs::write(&bad, b"not a png at all, just text bytes").unwrap();
        let text = |path: &std::path::Path| format!("hi [Image #1: {}]", path.display());
        let drawn = render_attachments(&text(&good), SETTINGS, BOUNDS);
        let plain = render_attachments(&text(&bad), SETTINGS, BOUNDS);
        let missing = render_attachments("[Image #1: /no/such.png]", SETTINGS, BOUNDS);
        let shown = std::fs::canonicalize(&good).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(drawn.contains("\x1b_G"));
        assert!(drawn.ends_with(&path_line(&shown)), "{drawn:?}");
        assert_eq!(plain, "");
        assert_eq!(missing, "");
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
    fn valid_png_renders_kitty_apc_row_newlines_then_path_line() {
        let dir = temp_dir("valid");
        let path = tall_png(&dir);
        let shown = std::fs::canonicalize(&path).unwrap();
        let out = render_image("t", &path.to_string_lossy(), SETTINGS, BOUNDS);
        std::fs::remove_dir_all(&dir).unwrap();
        let (cols, rows) = cell_grid((16, 64), None, BOUNDS.cols, BOUNDS.rows);
        assert!(out.starts_with("\x1b_G"));
        for key in ["f=100", "C=1", &format!("c={cols}"), &format!("r={rows}")] {
            assert!(out.contains(key), "{key} missing in {out:?}");
        }
        let image = out
            .strip_suffix(&path_line(&shown))
            .unwrap_or_else(|| panic!("no path line in {out:?}"));
        let body = image.trim_end_matches('\n');
        assert_eq!(image.len() - body.len(), rows as usize);
        assert!(body.ends_with("\x1b\\"));
    }

    #[test]
    fn tmux_png_renders_virtual_image_placeholder_rows_then_path_line() {
        let dir = temp_dir("tmux");
        let path = tall_png(&dir);
        let shown = std::fs::canonicalize(&path).unwrap();
        let tmux = ImageSettings {
            placement: Placement::Tmux,
            ..SETTINGS
        };
        let out = render_image("t", &path.to_string_lossy(), tmux, BOUNDS);
        std::fs::remove_dir_all(&dir).unwrap();
        let (_, rows) = cell_grid((16, 64), None, BOUNDS.cols, BOUNDS.rows);
        assert!(out.starts_with("\x1bPtmux;"));
        assert!(out.contains("U=1"));
        let image = out
            .strip_suffix(&path_line(&shown))
            .unwrap_or_else(|| panic!("no path line in {out:?}"));
        assert_eq!(image.matches('\n').count(), rows as usize);
        assert_eq!(image.matches('\u{10EEEE}').count() % rows as usize, 0);
    }

    #[test]
    fn drawn_path_line_shows_the_canonical_absolute_path() {
        let dir = temp_dir("canonical");
        let path = tall_png(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let dotted = dir.join("sub").join("..").join("tall.png");
        let shown = std::fs::canonicalize(&path).unwrap();
        let out = render_image("t", &dotted.to_string_lossy(), SETTINGS, BOUNDS);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(out.ends_with(&path_line(&shown)), "{out:?}");
        assert!(!shown.to_string_lossy().contains(".."));
    }

    fn visible(text: &str) -> String {
        let mut out = String::new();
        let mut rest = text;
        while let Some(start) = rest.find("\x1b]8;") {
            out.push_str(&rest[..start]);
            let after = &rest[start..];
            let end = after.find("\x1b\\").map_or(after.len(), |end| end + 2);
            rest = &after[end..];
        }
        out.push_str(rest);
        crate::frame::strip_ansi(&out)
    }

    #[test]
    fn path_line_link_bytes_add_no_width_or_wrap_in_rendered_markdown() {
        use unicode_width::UnicodeWidthStr;
        let dir = temp_dir(&"long-directory-name-".repeat(5));
        let path = tall_png(&dir);
        let shown = std::fs::canonicalize(&path).unwrap();
        let (highlighter, theme) = (
            crate::highlight::Highlighter::with_config(Default::default()).unwrap(),
            MarkdownTheme::default(),
        );
        let mut ctx = RenderContext::plain(&highlighter);
        ctx.terminal = true;
        ctx.images = Some(SETTINGS);
        let text = format!("see ![t]({}) after\n", path.display());
        let out = render_cell_display(&ctx, &text, &theme);
        std::fs::remove_dir_all(&dir).unwrap();
        let line = path_line(&shown);
        assert!(out.contains(&line), "{out:?}");
        let shown = shown.to_string_lossy();
        assert!(shown.width() > BOUNDS.cols as usize);
        let row = visible(line.trim_end_matches('\n'));
        assert_eq!(row, shown);
        assert_eq!(row.width(), shown.width());
    }

    #[test]
    fn placeholder_lines_wrap_as_one_line_of_cols_cells() {
        use unicode_width::UnicodeWidthStr;
        for cols in [80u32, 5] {
            let rows = placeholder_rows(3, cols, 3);
            for line in rows.lines() {
                let bare = line.replace("\x1b[38;5;3m", "").replace("\x1b[39m", "");
                let ranges = super::super::wrap::break_lines(&bare, 80, 80);
                assert_eq!(ranges, vec![0..bare.len()], "{bare:?}");
                assert_eq!(UnicodeWidthStr::width(bare.as_str()), cols as usize);
            }
        }
    }
}
