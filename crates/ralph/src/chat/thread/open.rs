use super::{ensure, open_ready, thread_dir, write_meta};
use ralph_core::chat_thread::{open_check, ThreadName};
use ralph_core::chat_thread_auto::{clear_closed, with_auto};
use ralph_core::chat_thread_cmd::OpenOpts;
use ralph_core::chat_thread_model::{build_model_args, with_model};
use std::io;
use std::path::{Path, PathBuf};

pub(crate) fn prepare_open(store: &Path, opts: &OpenOpts) -> Result<PathBuf, String> {
    let main = open_ready(store, &opts.name)?;
    if opts.auto.is_some() || opts.model.is_some() {
        let dir = thread_dir(&main, &opts.name);
        let failed = |e: io::Error| format!("could not open thread {}: {e}", opts.name);
        let meta = ensure(&main, &opts.name).map_err(failed)?;
        let with_auto = with_auto(&meta, opts.auto);
        let current = with_auto.as_ref().unwrap_or(&meta);
        if let Some(updated) = with_model(current, opts.model.as_deref()).or(with_auto) {
            write_meta(&dir, &updated).map_err(failed)?;
        }
    }
    Ok(main)
}

pub(in crate::chat) fn reopen(
    main: &Path,
    name: &ThreadName,
    model: Option<&str>,
) -> io::Result<Option<String>> {
    let meta = ensure(main, name)?;
    open_check(name, &meta).map_err(io::Error::other)?;
    let cleared = clear_closed(&meta);
    let updated = with_model(cleared.as_ref().unwrap_or(&meta), model).or(cleared);
    if let Some(updated) = &updated {
        write_meta(&thread_dir(main, name), updated)?;
    }
    Ok(updated.unwrap_or(meta).model)
}

pub(crate) fn model_args(
    store: &Path,
    thread: Option<&ThreadName>,
    model: Option<&str>,
) -> io::Result<Vec<String>> {
    thread.map_or(Ok(Vec::new()), |name| {
        let stored = reopen(&super::config_root(store), name, model)?;
        Ok(build_model_args(stored.as_deref()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::thread::{claim_closed, park_thread, pull_thread, read_meta, revive_thread};
    use ralph_core::chat_thread::{AutoMode, Merged, Parked, ThreadMeta, THREAD_FILE};

    fn name(s: &str) -> ThreadName {
        ThreadName::parse(s).unwrap()
    }

    fn model_opts(n: &str, model: Option<&str>) -> OpenOpts {
        OpenOpts {
            model: model.map(str::to_string),
            ..opts(n, None)
        }
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
                    landed: None,
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

    #[test]
    fn prepare_open_stores_a_model_and_a_later_model_overrides_it() {
        let root = tempfile::tempdir().unwrap();
        prepare_open(root.path(), &model_opts("t", Some("haiku"))).unwrap();
        assert_eq!(stored(root.path(), "t").model.as_deref(), Some("haiku"));
        prepare_open(root.path(), &model_opts("t", Some("opus"))).unwrap();
        assert_eq!(stored(root.path(), "t").model.as_deref(), Some("opus"));
        prepare_open(root.path(), &model_opts("t", None)).unwrap();
        assert_eq!(stored(root.path(), "t").model.as_deref(), Some("opus"));
    }

    #[test]
    fn prepare_open_stores_model_and_auto_together() {
        let root = tempfile::tempdir().unwrap();
        let both = OpenOpts {
            auto: Some(AutoMode::Drop),
            ..model_opts("t", Some("haiku"))
        };
        prepare_open(root.path(), &both).unwrap();
        let meta = stored(root.path(), "t");
        assert_eq!(meta.model.as_deref(), Some("haiku"));
        assert_eq!(meta.auto, Some(AutoMode::Drop));
    }

    #[test]
    fn prepare_open_refuses_a_model_change_while_the_thread_chat_is_open() {
        let root = tempfile::tempdir().unwrap();
        prepare_open(root.path(), &model_opts("busy", Some("opus"))).unwrap();
        let _claim = claim_closed(&thread_dir(root.path(), &name("busy")), &name("busy")).unwrap();
        assert!(prepare_open(root.path(), &model_opts("busy", Some("haiku"))).is_err());
        assert_eq!(stored(root.path(), "busy").model.as_deref(), Some("opus"));
    }

    #[test]
    fn reopen_returns_the_stored_model() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(reopen(root.path(), &name("plain"), None).unwrap(), None);
        prepare_open(root.path(), &model_opts("t", Some("haiku"))).unwrap();
        assert_eq!(
            reopen(root.path(), &name("t"), None).unwrap().as_deref(),
            Some("haiku")
        );
        assert_eq!(
            reopen(root.path(), &name("t"), Some("opus"))
                .unwrap()
                .as_deref(),
            Some("opus")
        );
        assert_eq!(stored(root.path(), "t").model.as_deref(), Some("opus"));
    }

    #[test]
    fn park_revive_and_pull_keep_the_model() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path();
        let n = name("t");
        prepare_open(main, &model_opts("t", Some("haiku"))).unwrap();
        park_thread(main, &n, &[]).unwrap();
        assert_eq!(stored(main, "t").model.as_deref(), Some("haiku"));
        revive_thread(main, &n).unwrap();
        assert_eq!(stored(main, "t").model.as_deref(), Some("haiku"));
        pull_thread(main, &n).unwrap();
        assert_eq!(stored(main, "t").model.as_deref(), Some("haiku"));
    }
}
