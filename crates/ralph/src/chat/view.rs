use crate::chat::inbox::Display;
use crate::render::image::resolve;
use crate::render::render_markdown;
use ralph_core::chat_view::{rebase_images, view_refusal, FileKind, ViewArgs, MAX_VIEW_BYTES};
use std::io::Read;
use std::path::{Path, PathBuf};

pub fn run(args: &ViewArgs, display: &Display) {
    let ViewArgs::File(typed) = args else {
        println!("{}", ralph_core::chat_view::VIEW_USAGE);
        return;
    };
    match load(&resolve(typed)) {
        Ok((text, dir)) => print!(
            "{}",
            render_markdown(&display.ctx, &rebase_images(&text, &dir), display.theme)
        ),
        Err(message) => println!("{message}"),
    }
}

fn kind(path: &Path) -> FileKind {
    match std::fs::metadata(path) {
        Err(_) => FileKind::Missing,
        Ok(meta) if meta.is_dir() => FileKind::Dir,
        Ok(meta) => FileKind::File { bytes: meta.len() },
    }
}

fn load(path: &Path) -> Result<(String, PathBuf), String> {
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
    Ok((text, dir))
}
