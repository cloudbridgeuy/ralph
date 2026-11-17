use super::{ensure, open_ready, thread_dir, write_meta};
use ralph_core::chat_thread::{open_check, ThreadName};
use ralph_core::chat_thread_auto::{clear_closed, with_auto};
use ralph_core::chat_thread_cmd::OpenOpts;
use std::io;
use std::path::{Path, PathBuf};

pub(super) fn prepare_open(store: &Path, opts: &OpenOpts) -> Result<PathBuf, String> {
    let main = open_ready(store, &opts.name)?;
    if opts.auto.is_some() {
        let dir = thread_dir(&main, &opts.name);
        let failed = |e: io::Error| format!("could not open thread {}: {e}", opts.name);
        let meta = ensure(&main, &opts.name).map_err(failed)?;
        if let Some(updated) = with_auto(&meta, opts.auto) {
            write_meta(&dir, &updated).map_err(failed)?;
        }
    }
    Ok(main)
}

pub(in crate::chat) fn reopen(main: &Path, name: &ThreadName) -> io::Result<()> {
    let meta = ensure(main, name)?;
    open_check(name, &meta).map_err(io::Error::other)?;
    if let Some(updated) = clear_closed(&meta) {
        write_meta(&thread_dir(main, name), &updated)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::thread::{claim_closed, read_meta};
    use ralph_core::chat_thread::{AutoMode, Merged, Parked, ThreadMeta, THREAD_FILE};

    fn name(s: &str) -> ThreadName {
        ThreadName::parse(s).unwrap()
    }

    fn opts(n: &str, auto: Option<AutoMode>) -> OpenOpts {
        OpenOpts {
            name: name(n),
            auto,
            model: None,
        }
    }

    fn stored(root: &Path, n: &str) -> ThreadMeta {
        read_meta(&thread_dir(root, &name(n))).unwrap()
    }

    #[test]
    fn prepare_open_stores_auto_on_a_fresh_thread() {
        for auto in [AutoMode::Merge, AutoMode::Drop] {
            let root = tempfile::tempdir().unwrap();
            prepare_open(root.path(), &opts("fresh", Some(auto))).unwrap();
            assert_eq!(stored(root.path(), "fresh").auto, Some(auto));
        }
    }

    #[test]
    fn prepare_open_overwrites_auto_on_an_existing_thread() {
        let root = tempfile::tempdir().unwrap();
        ensure(root.path(), &name("t")).unwrap();
        prepare_open(root.path(), &opts("t", Some(AutoMode::Merge))).unwrap();
        prepare_open(root.path(), &opts("t", Some(AutoMode::Drop))).unwrap();
        assert_eq!(stored(root.path(), "t").auto, Some(AutoMode::Drop));
    }

    #[test]
    fn prepare_open_without_auto_writes_nothing_new() {
        let root = tempfile::tempdir().unwrap();
        prepare_open(root.path(), &opts("new", None)).unwrap();
        assert!(!thread_dir(root.path(), &name("new"))
            .join(THREAD_FILE)
            .exists());
        ensure(root.path(), &name("old")).unwrap();
        prepare_open(root.path(), &opts("old", Some(AutoMode::Merge))).unwrap();
        prepare_open(root.path(), &opts("old", None)).unwrap();
        assert_eq!(stored(root.path(), "old").auto, Some(AutoMode::Merge));
    }

    #[test]
    fn prepare_open_refuses_merged_parked_and_in_use_without_writing() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path();
        for (n, mark) in [("merged", 0), ("parked", 1)] {
            let dir = thread_dir(main, &name(n));
            let mut meta = ensure(main, &name(n)).unwrap();
            if mark == 0 {
                meta.merged = Some(Merged {
                    at: "t".to_string(),
                    digest: "d".to_string(),
                });
            } else {
                meta.parked = Some(Parked {
                    at: "t".to_string(),
                });
            }
            write_meta(&dir, &meta).unwrap();
            assert!(prepare_open(main, &opts(n, Some(AutoMode::Drop))).is_err());
            assert_eq!(stored(main, n).auto, None);
        }
        ensure(main, &name("busy")).unwrap();
        let _claim = claim_closed(&thread_dir(main, &name("busy")), &name("busy")).unwrap();
        assert!(prepare_open(main, &opts("busy", Some(AutoMode::Drop))).is_err());
        assert_eq!(stored(main, "busy").auto, None);
    }
}
