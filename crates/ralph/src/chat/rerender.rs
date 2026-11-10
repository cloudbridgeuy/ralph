use crate::ansi::{DIM, RESET};
use crate::chat::inbox::Display;
use crate::chat::view;
use ralph_core::chat_rerender::reprint;
use ralph_server::chat::memory::Store;
use std::path::Path;

pub fn run(n: &Result<usize, String>, store: &Path, display: &Display) {
    let shown = n.clone().and_then(|n| {
        let (entries, _) = Store::new(store.to_path_buf())
            .load_log_quiet()
            .map_err(|error| format!("/rerender cannot read the chat log: {error}"))?;
        reprint(&entries, n)
    });
    match shown {
        Ok(reprint) => {
            println!("{DIM}{}{RESET}", reprint.header);
            print!("{}", view::render(display, &reprint.markdown));
        }
        Err(message) => println!("{message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_handler_takes_no_writer_and_only_a_store_path() {
        let _: fn(&Result<usize, String>, &Path, &Display) = run;
    }
}
