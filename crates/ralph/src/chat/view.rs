use crate::chat::inbox::Display;
use crate::chat::rex;
use crate::mermaid_guard;
use crate::render::image::{draw_png, ImageBounds};
use crate::render::{render_markdown_with, RenderContext};
use crate::ui_theme::UiColors;
use ralph_core::chat_view::{
    rebase_images, view_refusal, view_split_argv, view_split_notice, FileKind, ViewArgs, ViewHost,
    MAX_VIEW_BYTES,
};
use ralph_core::chunk::{ChunkType, ParsedChunk};
use ralph_core::kitty::ASSUMED_CELL_PX;
use ralph_core::mermaid_fit::Room;
use ralph_core::program_status::RexBlock;
use ralph_core::sanitize::strip_controls;
use ralph_server::disk_files::resolve;
use std::io::Read;
use std::path::{Path, PathBuf};

pub(crate) struct Viewed {
    pub text: String,
    pub file: PathBuf,
}

pub(super) fn rex_host(block: Option<RexBlock>, store: &Path) -> Option<ViewHost> {
    Some(ViewHost {
        block: block?,
        exe: std::env::current_exe().ok()?.to_string_lossy().into_owned(),
        cwd: std::env::current_dir().ok()?.to_string_lossy().into_owned(),
        store: std::path::absolute(store)
            .ok()?
            .to_string_lossy()
            .into_owned(),
    })
}

pub fn run(args: &ViewArgs, display: &Display, host: Option<&ViewHost>) {
    let ViewArgs::File(typed) = args else {
        println!("{}", ralph_core::chat_view::VIEW_USAGE);
        return;
    };
    match load(&resolve(typed)) {
        Ok(viewed) => show(&viewed, display, host),
        Err(message) => println!("{message}"),
    }
}

fn show(viewed: &Viewed, display: &Display, host: Option<&ViewHost>) {
    if let Some(host) = host {
        match rex::split(&view_split_argv(host, &viewed.file)) {
            Ok(()) => {
                println!("opened {} in a new Rex block", viewed.file.display());
                return;
            }
            Err(error) => println!("{}", view_split_notice(&error)),
        }
    }
    print!("{}", render(display, &viewed.text));
}

pub(crate) fn render(display: &Display, text: &str) -> String {
    render_markdown_with(&display.ctx, text, display.theme, |chunk| {
        picture(&display.ctx, chunk, &display.ui)
    })
}

fn picture(ctx: &RenderContext<'_>, chunk: &ParsedChunk, ui: &UiColors) -> Option<String> {
    let ChunkType::Code {
        language: Some(language),
    } = &chunk.chunk_type
    else {
        return None;
    };
    let images = ctx
        .images
        .filter(|_| ctx.terminal && language == "mermaid")?;
    let bounds = ImageBounds::current();
    let room = Room {
        cell_px: images.cell_px.unwrap_or(ASSUMED_CELL_PX),
        cols: bounds.cols,
        rows: bounds.rows,
    };
    let png = mermaid_guard::picture(&chunk.content, ctx.highlighter.theme(), ui, room)?;
    draw_png(&png, images, bounds)
}

fn kind(path: &Path) -> FileKind {
    match std::fs::metadata(path) {
        Err(_) => FileKind::Missing,
        Ok(meta) if meta.is_dir() => FileKind::Dir,
        Ok(meta) => FileKind::File { bytes: meta.len() },
    }
}

pub(crate) fn load(path: &Path) -> Result<Viewed, String> {
    if let Some(refusal) = view_refusal(path, kind(path)) {
        return Err(refusal);
    }
    let cannot = |error: std::io::Error| format!("/view cannot read {}: {error}", path.display());
    let mut text = String::new();
    std::fs::File::open(path)
        .map_err(cannot)?
        .take(MAX_VIEW_BYTES)
        .read_to_string(&mut text)
        .map_err(cannot)?;
    let absolute = std::path::absolute(path).map_err(cannot)?;
    let dir = absolute.parent().map(Path::to_path_buf).unwrap_or_default();
    Ok(Viewed {
        text: rebase_images(&strip_controls(&text), &dir),
        file: absolute,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::{Highlighter, ThemeConfig};
    use crate::markdown::MarkdownTheme;
    use crate::render::render_markdown;

    #[test]
    fn code_fences_highlight_like_chat_replies() {
        let highlighter = Highlighter::with_config(ThemeConfig::default()).unwrap();
        let theme = MarkdownTheme::default();
        let display = Display {
            ctx: RenderContext::terminal(&highlighter),
            theme: &theme,
            color: true,
            ui: UiColors::default(),
        };
        let ctx = &display.ctx;
        let text = "# Notes\n\n```rust\nfn main() {}\n```\n\n```sh\necho hi\n```\n\n```diff\n-old\n+new\n```\n";
        let walked = render(&display, text);
        assert_eq!(walked, render_markdown(ctx, text, &theme));
        let (rust, _) = walked.split_once("── sh ").unwrap();
        assert!(rust.contains("\x1b[38;"), "{rust:?}");
        assert!(!rust.contains("fn main() {}"), "{rust:?}");
    }

    fn loaded(source: &str) -> String {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doc.md");
        std::fs::write(&path, source).unwrap();
        load(&path).unwrap().text
    }

    #[test]
    fn a_viewed_file_loses_its_control_bytes() {
        let text = loaded("a\x1b]0;t\x07b\r\n\tc\u{9b}2J\x7f\n");
        assert_eq!(text, "a]0;tb\n\tc2J\n");
    }

    #[test]
    fn text_after_an_unfinished_escape_stays() {
        let text = loaded("head\x1b_Gunfinished\n\ntail\n");
        assert!(text.ends_with("unfinished\n\ntail\n"), "{text:?}");
    }

    #[test]
    fn toml_and_ts_fences_highlight_in_view() {
        let highlighter = Highlighter::with_config(ThemeConfig::default()).unwrap();
        let theme = MarkdownTheme::default();
        let display = Display {
            ctx: RenderContext::terminal(&highlighter),
            theme: &theme,
            color: true,
            ui: UiColors::default(),
        };
        let text = "```toml\nname = \"ralph\"\n```\n\n```ts\nconst n: number = 1;\n```\n";
        let walked = render(&display, text);
        assert_eq!(walked, render_markdown(&display.ctx, text, &theme));
        let (toml, ts) = walked.split_once("── ts ").unwrap();
        assert!(!toml.contains("name = \"ralph\""), "{toml:?}");
        assert!(!ts.contains("const n: number = 1;"), "{ts:?}");
    }
}
