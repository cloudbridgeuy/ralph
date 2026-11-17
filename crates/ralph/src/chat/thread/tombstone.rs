use ralph_core::chat_thread::{ThreadMeta, ThreadName, THREADS_DIR};
use ralph_core::chat_thread_log::{tombstone_line, DROPPED_FILE};
use ralph_server::private_file::check_path;
use ralph_server::warn::warn_if_err;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

pub(super) fn record_drop(main: &Path, name: &ThreadName, meta: &ThreadMeta) {
    let line = tombstone_line(name, meta, &chrono::Utc::now().to_rfc3339());
    warn_if_err(
        append(main, &line),
        format!("Failed to record the dropped thread {name}"),
    );
}

fn append(main: &Path, line: &str) -> io::Result<()> {
    let dir = main.join(THREADS_DIR);
    fs::create_dir_all(&dir)?;
    let path = dir.join(DROPPED_FILE);
    check_path(&path)?;
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)?;
    let prefix = separator(last_byte(&mut file)?);
    file.write_all(format!("{prefix}{line}").as_bytes())
}

fn separator(last: Option<u8>) -> &'static str {
    match last {
        Some(byte) if byte != b'\n' => "\n",
        _ => "",
    }
}

fn last_byte(file: &mut File) -> io::Result<Option<u8>> {
    if file.metadata()?.len() == 0 {
        return Ok(None);
    }
    file.seek(SeekFrom::End(-1))?;
    let mut byte = [0u8];
    file.read_exact(&mut byte)?;
    Ok(Some(byte[0]))
}

#[cfg(test)]
mod tests {
    use super::separator;
    use crate::chat::thread::{drop_thread, ensure, open_names, rows, thread_dir};
    use ralph_core::chat_thread::{ThreadName, THREADS_DIR};
    use ralph_core::chat_thread_log::{parse_tombstones, DROPPED_FILE};
    use ralph_server::chat::memory::Store;
    use std::path::Path;

    fn name(s: &str) -> ThreadName {
        ThreadName::parse(s).unwrap()
    }

    fn dropped(main: &Path) -> String {
        std::fs::read_to_string(main.join(THREADS_DIR).join(DROPPED_FILE)).unwrap_or_default()
    }

    #[test]
    fn a_drop_deletes_the_folder_and_leaves_one_tombstone() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("chat");
        let meta = ensure(&main, &name("auth")).unwrap();
        drop_thread(&main, &name("auth"), &[]).unwrap();
        assert!(!thread_dir(&main, &name("auth")).exists());
        let text = dropped(&main);
        assert_eq!(text.lines().count(), 1);
        let tombstones = parse_tombstones(&text);
        assert_eq!(tombstones[0].name, "auth");
        assert_eq!(tombstones[0].base, meta.base);
        assert_eq!(tombstones[0].created, meta.created);
    }

    #[test]
    fn later_drops_append_in_order() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("chat");
        for thread in ["one", "two"] {
            ensure(&main, &name(thread)).unwrap();
            drop_thread(&main, &name(thread), &[]).unwrap();
        }
        let names: Vec<_> = parse_tombstones(&dropped(&main))
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, vec!["one", "two"]);
    }

    #[test]
    fn the_tombstone_file_is_not_a_thread() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("chat");
        ensure(&main, &name("keep")).unwrap();
        ensure(&main, &name("gone")).unwrap();
        drop_thread(&main, &name("gone"), &[]).unwrap();
        let listed = rows(&main, &|_| 0).unwrap();
        let names: Vec<&str> = listed.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(names, vec!["keep"]);
        assert_eq!(open_names(&main), vec!["keep"]);
    }

    #[test]
    fn a_refused_drop_writes_nothing() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("chat");
        ensure(&main, &name("auth")).unwrap();
        assert!(drop_thread(&main, &name("auth"), &["w1".to_string()]).is_err());
        let held = Store::new(thread_dir(&main, &name("auth")))
            .claim_chat()
            .unwrap()
            .unwrap();
        assert!(drop_thread(&main, &name("auth"), &[]).is_err());
        drop(held);
        assert!(!main.join(THREADS_DIR).join(DROPPED_FILE).exists());
        assert!(thread_dir(&main, &name("auth")).exists());
    }

    #[test]
    fn a_drop_after_a_torn_last_line_starts_a_new_line() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("chat");
        let threads = main.join(THREADS_DIR);
        std::fs::create_dir_all(&threads).unwrap();
        std::fs::write(threads.join(DROPPED_FILE), "{\"name\":\"old").unwrap();
        ensure(&main, &name("new")).unwrap();
        drop_thread(&main, &name("new"), &[]).unwrap();
        let text = dropped(&main);
        assert_eq!(text.lines().count(), 2, "{text}");
        assert_eq!(text.lines().next(), Some("{\"name\":\"old"));
        let names: Vec<_> = parse_tombstones(&text)
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, vec!["new"]);
    }

    #[test]
    fn a_line_break_is_needed_only_after_an_unterminated_line() {
        assert_eq!(separator(None), "");
        assert_eq!(separator(Some(b'\n')), "");
        assert_eq!(separator(Some(b'}')), "\n");
    }

    #[test]
    fn dropping_an_unknown_thread_writes_nothing() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("chat");
        assert!(drop_thread(&main, &name("ghost"), &[]).is_err());
        assert_eq!(dropped(&main), "");
    }
}
