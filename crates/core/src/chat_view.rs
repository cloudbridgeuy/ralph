use crate::chat_hold::{command_args, HoldCompletion};
use crate::kitty::{parse_image_markers, Segment};
use crate::program_status::{rex_split_argv, RexBlock, RexSplit, Stdout};
use std::ops::Range;
use std::path::Path;

const VIEW: &str = "/view";
pub const VIEW_USAGE: &str = "Usage: /view <file.md>";
pub const MAX_VIEW_BYTES: u64 = 2 * 1024 * 1024;
const MARKDOWN_EXTENSIONS: [&str; 2] = ["md", "markdown"];
const DEFAULT_PAGER: &str = "less";
const DEFAULT_PAGER_ARG: &str = "-r";
const NO_PAGER: &str = "none";
const CTRL_C: u8 = 0x03;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PagerCommand {
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewPager {
    Direct,
    Command(PagerCommand),
}

fn default_pager() -> ViewPager {
    ViewPager::Command(PagerCommand {
        program: DEFAULT_PAGER.to_string(),
        args: vec![DEFAULT_PAGER_ARG.to_string()],
    })
}

pub fn resolve_view_pager(config: Option<&str>) -> (ViewPager, Option<String>) {
    let Some(configured) = config else {
        return (default_pager(), None);
    };
    let mut words = configured.split_whitespace().map(str::to_string);
    match words.next() {
        None => (
            default_pager(),
            Some(format!(
                "[chat] view_pager is empty; using \"{DEFAULT_PAGER} {DEFAULT_PAGER_ARG}\""
            )),
        ),
        Some(_) if configured.trim() == NO_PAGER => (ViewPager::Direct, None),
        Some(program) => (
            ViewPager::Command(PagerCommand {
                program,
                args: words.collect(),
            }),
            None,
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Show<'a> {
    Print,
    PrintAndWait,
    Page(&'a PagerCommand),
}

pub fn show_plan(pager: &ViewPager, stdin_tty: bool, stdout: Stdout) -> Show<'_> {
    match (stdout, pager) {
        (Stdout::NotTty, _) => Show::Print,
        (Stdout::Tty, ViewPager::Command(command)) => Show::Page(command),
        (Stdout::Tty, ViewPager::Direct) if stdin_tty => Show::PrintAndWait,
        (Stdout::Tty, ViewPager::Direct) => Show::Print,
    }
}

pub fn quits(key: &[u8]) -> bool {
    matches!(key.first(), Some(&(b'q' | CTRL_C)))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ViewTarget {
    #[default]
    Inline,
    RexPane,
}

pub fn resolve_view_target(config: Option<&str>) -> (ViewTarget, Option<String>) {
    match config.map(str::trim) {
        None | Some("inline") => (ViewTarget::Inline, None),
        Some("rex-pane") => (ViewTarget::RexPane, None),
        Some(other) => (
            ViewTarget::Inline,
            Some(format!(
                "[chat] view = {other:?} is not \"inline\" or \"rex-pane\"; using \"inline\""
            )),
        ),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewHost {
    pub block: RexBlock,
    pub exe: String,
    pub cwd: String,
    pub store: String,
}

pub fn view_host(target: ViewTarget, stdout: Stdout, found: Option<ViewHost>) -> Option<ViewHost> {
    found.filter(|_| target == ViewTarget::RexPane && stdout == Stdout::Tty)
}

pub fn view_split_argv(host: &ViewHost, file: &Path) -> Vec<String> {
    let name = file.file_name().unwrap_or(file.as_os_str());
    let label = format!("view {}", name.to_string_lossy());
    let file = file.to_string_lossy();
    let command = [&host.exe, "view", "--store", &host.store, &file].map(str::to_string);
    let split = RexSplit {
        block: &host.block,
        label: &label,
        cwd: &host.cwd,
    };
    rex_split_argv(split, command.to_vec())
}

pub fn view_split_notice(error: &str) -> String {
    let first = error.lines().next().unwrap_or_default();
    format!("/view: {first}; showing inline")
}

#[cfg(test)]
mod tests;
