use super::{read_meta, thread_dir};
use ralph_core::chat_thread::{ThreadMeta, ThreadName, THREADS_DIR};
use ralph_core::chat_thread_graph::{graph_lines, graph_opts, lanes_from, GraphOpts};
use ralph_core::chat_thread_log::{parse_tombstones, Tombstone, DROPPED_FILE};
use ralph_server::chat::memory::Store;
use std::fs;
use std::io::{self, IsTerminal};
use std::path::Path;

pub(in crate::chat) fn print_log(main: &Path, columns: usize) -> io::Result<()> {
    let color = io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    println!("{}", log_text(main, &graph_opts(color, columns))?);
    Ok(())
}

fn log_text(main: &Path, opts: &GraphOpts) -> io::Result<String> {
    let head = Store::new(main.to_path_buf()).log_len()?.saturating_sub(1);
    let lanes = lanes_from(&live_threads(main), &tombstones(main));
    Ok(graph_lines(head, &lanes, opts).join("\n"))
}

fn live_threads(main: &Path) -> Vec<(String, ThreadMeta)> {
    let Ok(entries) = fs::read_dir(main.join(THREADS_DIR)) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| ThreadName::parse(entry.ok()?.file_name().to_str()?).ok())
        .filter_map(|name| {
            let meta = read_meta(&thread_dir(main, &name)).ok()?;
            Some((name.to_string(), meta))
        })
        .collect()
}

fn tombstones(main: &Path) -> Vec<Tombstone> {
    let text = fs::read_to_string(main.join(THREADS_DIR).join(DROPPED_FILE)).unwrap_or_default();
    parse_tombstones(&text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::thread::{drop_thread, ensure, merge_thread, park_thread};
    use ralph_core::chat_memory::Role;

    fn name(s: &str) -> ThreadName {
        ThreadName::parse(s).unwrap()
    }

    #[test]
    fn a_store_with_no_threads_prints_only_the_trunk() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("chat");
        let text = log_text(&main, &graph_opts(false, 80)).unwrap();
        assert_eq!(text, "●  #0 main");
    }

    #[test]
    fn every_thread_state_shows_and_a_dropped_thread_stays_as_a_tombstone() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("chat");
        let store = Store::new(main.clone());
        store.append(Role::User, "one").unwrap();
        store.append(Role::User, "two").unwrap();
        for thread in ["a", "b", "c", "d"] {
            ensure(&main, &name(thread)).unwrap();
        }
        Store::new(thread_dir(&main, &name("a")))
            .append(Role::User, "side")
            .unwrap();
        let mut curate = |_: &str| Ok::<String, String>("Picked it. #a:0".to_string());
        merge_thread(&main, &name("a"), &[], &mut curate).unwrap();
        park_thread(&main, &name("b"), &[]).unwrap();
        drop_thread(&main, &name("c"), &[]).unwrap();
        let text = log_text(&main, &graph_opts(false, 80)).unwrap();
        assert!(text.contains("b  parked"), "{text}");
        assert!(text.contains("c  dropped"), "{text}");
        assert!(text.contains("d  open"), "{text}");
        assert!(text.contains("a  merged, landing unknown"), "{text}");
        assert!(text.contains("#2 main"), "{text}");
    }
}
