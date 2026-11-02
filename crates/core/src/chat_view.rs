use crate::chat_hold::{command_args, HoldCompletion};
use crate::kitty::{parse_image_markers, Segment};
use std::ops::Range;
use std::path::Path;

const VIEW: &str = "/view";
pub const VIEW_USAGE: &str = "Usage: /view <file.md>";
pub const MAX_VIEW_BYTES: u64 = 2 * 1024 * 1024;
const MARKDOWN_EXTENSIONS: [&str; 2] = ["md", "markdown"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewArgs {
    File(String),
    Usage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Missing,
    Dir,
    File { bytes: u64 },
}

pub fn parse_view_args(args: &str) -> ViewArgs {
    match args.trim() {
        "" => ViewArgs::Usage,
        path => ViewArgs::File(path.to_string()),
    }
}

fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            MARKDOWN_EXTENSIONS
                .iter()
                .any(|known| extension.eq_ignore_ascii_case(known))
        })
}

pub fn view_refusal(path: &Path, kind: FileKind) -> Option<String> {
    let shown = path.display();
    match kind {
        FileKind::Missing => Some(format!("/view: no such file: {shown}")),
        FileKind::Dir => Some(format!("/view: {shown} is a directory")),
        _ if !is_markdown(path) => Some(format!("/view shows .md and .markdown files: {shown}")),
        FileKind::File { bytes } if bytes > MAX_VIEW_BYTES => Some(format!(
            "/view: {shown} is {bytes} bytes; the limit is {MAX_VIEW_BYTES}"
        )),
        FileKind::File { .. } => None,
    }
}

fn rebase(path: &str, dir: &Path) -> String {
    if path.starts_with('/') || path.starts_with('~') {
        path.to_string()
    } else {
        dir.join(path).display().to_string()
    }
}

pub fn rebase_images(text: &str, dir: &Path) -> String {
    parse_image_markers(text)
        .into_iter()
        .map(|segment| match segment {
            Segment::Text(text) => text,
            Segment::Image { alt, path } => format!("![{alt}]({})", rebase(&path, dir)),
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathQuery {
    pub span: Range<usize>,
    pub dir: String,
    pub prefix: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

pub fn view_path_query(line: &str, cursor: usize) -> Option<PathQuery> {
    let typed = command_args(line, cursor, VIEW)?.trim_start();
    let split = typed.rfind('/').map_or(0, |slash| slash + 1);
    Some(PathQuery {
        span: cursor - typed.len()..cursor,
        dir: typed[..split].to_string(),
        prefix: typed[split..].to_string(),
    })
}

pub fn view_path_completion(query: &PathQuery, entries: &[DirEntry]) -> HoldCompletion {
    let shows_hidden = query.prefix.starts_with('.');
    let mut values: Vec<String> = entries
        .iter()
        .filter(|entry| entry.name.starts_with(&query.prefix))
        .filter(|entry| shows_hidden || !entry.name.starts_with('.'))
        .filter(|entry| entry.is_dir || is_markdown(Path::new(&entry.name)))
        .map(|entry| {
            let slash = if entry.is_dir { "/" } else { "" };
            format!("{}{}{slash}", query.dir, entry.name)
        })
        .collect();
    values.sort();
    HoldCompletion {
        span: query.span.clone(),
        values,
    }
}

#[cfg(test)]
mod tests;
