use super::{read_meta, thread_dir, write_meta};
use ralph_core::chat_thread::ThreadName;
use ralph_core::chat_thread_log::with_landing;
use std::path::Path;

pub(in crate::chat) fn set_landed(main: &Path, name: &ThreadName, id: usize) -> Result<(), String> {
    let dir = thread_dir(main, name);
    let failed = |e: std::io::Error| format!("could not record where thread {name} landed: {e}");
    let meta = read_meta(&dir).map_err(failed)?;
    match with_landing(&meta, id) {
        Some(patched) => write_meta(&dir, &patched).map_err(failed),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::chat_thread::{Merged, ThreadMeta};

    fn seed(main: &Path, name: &ThreadName, merged: Option<Option<usize>>) {
        let dir = thread_dir(main, name);
        std::fs::create_dir_all(&dir).unwrap();
        let meta = ThreadMeta {
            base: 2,
            created: "c".to_string(),
            merged: merged.map(|landed| Merged {
                at: "t".to_string(),
                digest: "d".to_string(),
                landed,
            }),
            parked: None,
            auto: None,
            model: None,
            closed: false,
        };
        write_meta(&dir, &meta).unwrap();
    }

    fn landed(main: &Path, name: &ThreadName) -> Option<Option<usize>> {
        let meta = read_meta(&thread_dir(main, name)).unwrap();
        meta.merged.map(|merged| merged.landed)
    }

    #[test]
    fn a_merged_thread_records_its_landing() {
        let dir = tempfile::tempdir().unwrap();
        let name = ThreadName::parse("a").unwrap();
        seed(dir.path(), &name, Some(None));
        set_landed(dir.path(), &name, 7).unwrap();
        assert_eq!(landed(dir.path(), &name), Some(Some(7)));
    }

    #[test]
    fn an_unmerged_thread_is_not_touched() {
        let dir = tempfile::tempdir().unwrap();
        let name = ThreadName::parse("a").unwrap();
        seed(dir.path(), &name, None);
        set_landed(dir.path(), &name, 7).unwrap();
        assert_eq!(landed(dir.path(), &name), None);
    }

    #[test]
    fn a_known_landing_stays() {
        let dir = tempfile::tempdir().unwrap();
        let name = ThreadName::parse("a").unwrap();
        seed(dir.path(), &name, Some(Some(3)));
        set_landed(dir.path(), &name, 7).unwrap();
        assert_eq!(landed(dir.path(), &name), Some(Some(3)));
    }

    #[test]
    fn a_missing_thread_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let name = ThreadName::parse("a").unwrap();
        assert!(set_landed(dir.path(), &name, 7).is_err());
    }
}
