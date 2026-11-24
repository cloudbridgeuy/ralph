use crate::ansi::{DIM, RESET};
use crate::chat::inbox::Display;
use crate::chat::view;
use ralph_core::chat_rerender::{reprint, Reprint};
use ralph_server::chat::memory::Store;
use std::path::Path;

fn reprint_of(n: usize, store: &Path) -> Result<Reprint, String> {
    let (entries, _) = Store::new(store.to_path_buf())
        .load_log_quiet()
        .map_err(|error| format!("/rerender cannot read the chat log: {error}"))?;
    reprint(&entries, n)
}

pub(super) fn redraw_text(store: &Path, display: &Display, cols: usize) -> Option<String> {
    let reprint = reprint_of(1, store).ok()?;
    Some(format!(
        "{DIM}re-rendered at {cols} cols{RESET}\n{}",
        view::render(display, &reprint.markdown)
    ))
}

pub fn run(n: &Result<usize, String>, store: &Path, display: &Display) {
    match n.clone().and_then(|n| reprint_of(n, store)) {
        Ok(reprint) => {
            println!("{DIM}{}{RESET}", reprint.header);
            print!("{}", view::render(display, &reprint.markdown));
        }
        Err(message) => println!("{message}"),
    }
}
