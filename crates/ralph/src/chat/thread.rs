use crate::chat::memory::Store;
use crate::cli::{ThreadArgs, ThreadCommand, NAP_TIMEOUT_SECS};
use crate::private_file::check_path;
use crate::worker_mcp::Inbox;
use crate::workers::{canonical_store, running_workers};
use crate::{run_curator, Curator};
use ralph_core::chat::VIEW_LINES;
use ralph_core::chat_memory::{view, CallKind, MemoryCall, ViewSplit};
use ralph_core::chat_thread::{
    digest_with, fork_check, idle_check, label_item, merge_prompt, merged_check, merged_report,
    thread_split, Merged, Scope, ThreadMeta, ThreadName, MAIN_PREFIX_LINES, THREADS_DIR,
    THREAD_FILE,
};
use ralph_core::worker::{chat_message, INBOX_DIR};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Default)]
pub struct Reach<'a> {
    pub limit: Option<usize>,
    pub label: Option<&'a ThreadName>,
}

pub struct Fork {
    pub main: Store,
    pub name: ThreadName,
    pub meta: ThreadMeta,
}

pub struct Memory {
    own: Store,
    fork: Option<Fork>,
}

impl Memory {
    pub fn open(dir: &Path) -> Result<Memory, String> {
        Ok(Memory {
            own: Store::new(dir.to_path_buf()),
            fork: open_fork(dir)?,
        })
    }

    pub fn call(&self, call: &MemoryCall) -> Result<String, String> {
        let own = |label| self.own.call(&call.tool, Reach { limit: None, label });
        match (&call.scope, &self.fork) {
            (Scope::Own | Scope::Main, None) => own(None),
            (Scope::Own, Some(fork)) => own(Some(&fork.name)),
            (Scope::Thread(name), Some(fork)) if *name == fork.name => own(Some(name)),
            (Scope::Main, Some(fork)) => {
                fork_check(&call.tool, fork.meta.base)?;
                let reach = Reach {
                    limit: Some(fork.meta.base),
                    label: None,
                };
                fork.main.call(&call.tool, reach)
            }
            (Scope::Thread(name), fork) => {
                let main = fork.as_ref().map_or(&self.own, |fork| &fork.main);
                let dir = thread_dir(main.dir(), name);
                if !dir.join(THREAD_FILE).is_file() {
                    return Err(format!("no thread {name}; `ralph thread ls` lists them."));
                }
                let reach = Reach {
                    limit: None,
                    label: Some(name),
                };
                Store::new(dir).call(&call.tool, reach)
            }
        }
    }

    pub fn dir(&self) -> &Path {
        self.own.dir()
    }
}

fn fork_of(store: &Path) -> Option<(PathBuf, ThreadName)> {
    let threads = store.parent()?;
    if threads.file_name()? != THREADS_DIR {
        return None;
    }
    let name = ThreadName::parse(store.file_name()?.to_str()?).ok()?;
    Some((threads.parent()?.to_path_buf(), name))
}

pub fn open_fork(store: &Path) -> Result<Option<Fork>, String> {
    let Some((main, name)) = fork_of(store) else {
        return Ok(None);
    };
    let meta = read_meta(store).map_err(|e| format!("thread {name}: {e}"))?;
    Ok(Some(Fork {
        main: Store::new(main),
        name,
        meta,
    }))
}

pub fn fork_view(fork: &Fork, own: ViewSplit) -> io::Result<ViewSplit> {
    let (entries, tree) = fork.main.load()?;
    let base = fork.meta.base.min(entries.len());
    let prefix = view(&entries[..base], &tree, MAIN_PREFIX_LINES);
    Ok(thread_split(prefix, fork.meta.base, &fork.name, own))
}

pub fn run(args: ThreadArgs) -> Result<(), Box<dyn std::error::Error>> {
    let main = super::store_dir(args.store)?;
    match args.command {
        ThreadCommand::Ls => {
            let rows = list(&main)?;
            println!("{}", if rows.is_empty() { "no threads" } else { &rows });
        }
        ThreadCommand::Drop { name } => {
            drop_thread(&main, &name, &running_ids(&main, &name))?;
            println!("dropped thread {name}");
        }
        ThreadCommand::Merge { name } => {
            let dir = thread_dir(&main, &name);
            let curator = Curator {
                kind: CallKind::Merge,
                store: &dir,
                run: None,
                timeout: NAP_TIMEOUT_SECS,
            };
            let running = running_ids(&main, &name);
            let message = merge_thread(&main, &name, &running, &mut |prompt: &str| {
                run_curator(prompt, &curator)
            })?;
            println!("merged thread {name} into main's inbox\n{message}");
        }
    }
    Ok(())
}

fn running_ids(main: &Path, name: &ThreadName) -> Vec<String> {
    running_workers(&canonical_store(thread_dir(main, name)))
        .into_iter()
        .map(|worker| worker.id)
        .collect()
}

fn read_meta(dir: &Path) -> io::Result<ThreadMeta> {
    let text = fs::read_to_string(dir.join(THREAD_FILE))?;
    serde_json::from_str(&text).map_err(io::Error::other)
}

pub fn config_root(store: &Path) -> PathBuf {
    fork_of(store).map_or_else(|| store.to_path_buf(), |(main, _)| main)
}

pub fn thread_dir(main: &Path, name: &ThreadName) -> PathBuf {
    main.join(THREADS_DIR).join(name.as_str())
}

pub fn ensure(main: &Path, name: &ThreadName) -> io::Result<ThreadMeta> {
    let dir = thread_dir(main, name);
    match read_meta(&dir) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        known => return known,
    }
    let meta = ThreadMeta {
        base: Store::new(main.to_path_buf()).log_len()?,
        created: chrono::Utc::now().to_rfc3339(),
        merged: None,
    };
    fs::create_dir_all(&dir)?;
    write_meta(&dir, &meta)?;
    Ok(meta)
}

fn write_meta(dir: &Path, meta: &ThreadMeta) -> io::Result<()> {
    let path = dir.join(THREAD_FILE);
    check_path(&path)?;
    fs::write(path, serde_json::to_string(meta).map_err(io::Error::other)?)
}

pub fn merge_thread(
    main: &Path,
    name: &ThreadName,
    running: &[String],
    curate: &mut impl FnMut(&str) -> Result<String, String>,
) -> Result<String, String> {
    let dir = thread_dir(main, name);
    let meta = known_meta(&dir, name)?;
    merged_check(name, &meta)?;
    idle_check(name, running)?;
    let _claim = claim_closed(&dir, name)?;
    let failed = |e: io::Error| format!("could not merge thread {name}: {e}");
    let (entries, tree) = Store::new(dir.clone()).load().map_err(failed)?;
    let lines: Vec<String> = view(&entries, &tree, VIEW_LINES)
        .iter()
        .map(|line| label_item(line, name))
        .collect();
    let digest = digest_with(&merge_prompt(name, meta.base, &lines), name, curate)
        .map_err(|e| format!("thread {name} was not merged: {e}"))?;
    let at = chrono::Utc::now().to_rfc3339();
    let report = merged_report(name, &digest, &at);
    let merged = ThreadMeta {
        merged: Some(Merged { at, digest }),
        ..meta
    };
    write_meta(&dir, &merged).map_err(failed)?;
    Inbox::new(main.join(INBOX_DIR))
        .post(&report, 0)
        .map_err(|e| {
            format!(
                "thread {name} is merged, but its digest did not reach main's inbox: {e}. The digest is in {}.",
                dir.join(THREAD_FILE).display()
            )
        })?;
    Ok(chat_message(&[report]))
}

fn known_meta(dir: &Path, name: &ThreadName) -> Result<ThreadMeta, String> {
    if !dir.join(THREAD_FILE).is_file() {
        return Err(format!("no thread {name}; `ralph thread ls` lists them."));
    }
    read_meta(dir).map_err(|e| format!("thread {name}: {e}"))
}

fn claim_closed(dir: &Path, name: &ThreadName) -> Result<fs::File, String> {
    Store::new(dir.to_path_buf())
        .claim_chat()
        .map_err(|e| format!("could not claim thread {name}: {e}"))?
        .map_err(|holder| {
            let by = holder
                .map(|pid| format!(" by pid {pid}"))
                .unwrap_or_default();
            format!("thread {name} is in use{by}; close its chat first.")
        })
}

pub fn list(main: &Path) -> io::Result<String> {
    let entries = match fs::read_dir(main.join(THREADS_DIR)) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(String::new()),
        entries => entries?,
    };
    let mut names: Vec<ThreadName> = entries
        .filter_map(|entry| ThreadName::parse(entry.ok()?.file_name().to_str()?).ok())
        .collect();
    names.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    let mut rows = Vec::new();
    for name in names {
        let dir = thread_dir(main, &name);
        let Ok(meta) = read_meta(&dir) else {
            continue;
        };
        let messages = Store::new(dir).log_len()?;
        let merged = meta
            .merged
            .map(|merged| format!("  merged {}", merged.at))
            .unwrap_or_default();
        rows.push(format!(
            "{name}  forked at #{}  {messages} messages  {}{merged}",
            meta.base, meta.created
        ));
    }
    Ok(rows.join("\n"))
}

pub fn drop_thread(main: &Path, name: &ThreadName, running: &[String]) -> Result<(), String> {
    let dir = thread_dir(main, name);
    merged_check(name, &known_meta(&dir, name)?)?;
    idle_check(name, running)?;
    let _claim = claim_closed(&dir, name)?;
    fs::remove_dir_all(&dir).map_err(|e| format!("could not drop thread {name}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_for_chat;
    use ralph_core::chat_memory::{parse_tool_call, Role};
    use serde_json::json;
    use std::fs;

    fn name(s: &str) -> ThreadName {
        ThreadName::parse(s).unwrap()
    }

    fn call(memory: &Memory, tool: &str, args: serde_json::Value) -> Result<String, String> {
        memory.call(&parse_tool_call(tool, &args).unwrap())
    }

    fn forked() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("chat");
        let store = Store::new(main.clone());
        store.append(Role::User, "main one needle").unwrap();
        store.append(Role::Assistant, "main two").unwrap();
        let auth = name("auth");
        assert_eq!(ensure(&main, &auth).unwrap().base, 2);
        store.append(Role::User, "main after fork needle").unwrap();
        let dir = thread_dir(&main, &auth);
        Store::new(dir.clone())
            .append(Role::User, "thread one needle")
            .unwrap();
        (root, main, dir)
    }

    #[test]
    fn a_thread_lives_under_main_and_keeps_its_fork_point() {
        let (_root, main, dir) = forked();
        assert_eq!(dir, main.join("threads").join("auth"));
        assert_eq!(ensure(&main, &name("auth")).unwrap().base, 2);
        assert_eq!(config_root(&dir), main);
        assert_eq!(config_root(&main), main);
    }

    #[test]
    fn a_thread_reads_its_own_ids_labelled_and_main_up_to_the_fork() {
        let (_root, _main, dir) = forked();
        let memory = Memory::open(&dir).unwrap();
        let own = call(&memory, "zoom", json!({"id": "auth:0"})).unwrap();
        assert!(own.starts_with("#auth:0 user"), "{own}");
        let main = call(&memory, "zoom", json!({"id": "1"})).unwrap();
        assert!(main.starts_with("#1 assistant"), "{main}");
        assert_eq!(
            call(&memory, "zoom", json!({"id": "2"})).unwrap_err(),
            "#2 is after this thread's fork point (#2); pull to see it."
        );
        let page = call(&memory, "read", json!({"id": "auth:0"})).unwrap();
        assert!(page.starts_with("#auth:0 · user"), "{page}");
        let recalled = call(&memory, "recall", json!({"pattern": "needle"})).unwrap();
        assert!(recalled.starts_with("#auth:0 · user"), "{recalled}");
        let in_main = call(
            &memory,
            "recall",
            json!({"pattern": "needle", "thread": "main"}),
        )
        .unwrap();
        assert!(
            in_main.starts_with("#0 · user") && !in_main.contains("after fork"),
            "{in_main}"
        );
        let found = call(
            &memory,
            "search",
            json!({"query": "needle", "thread": "main"}),
        )
        .unwrap();
        assert!(!found.contains("#2 "), "{found}");
    }

    #[test]
    fn a_thread_view_shows_main_up_to_the_fork_then_its_own_lines() {
        let (_root, _main, dir) = forked();
        let fork = open_fork(&dir).unwrap().unwrap();
        let own = ViewSplit {
            tail: vec!["#0 user x thread one needle".to_string()],
            ..ViewSplit::default()
        };
        let split = fork_view(&fork, own).unwrap();
        assert_eq!(split.older.len(), 4, "{:?}", split.older);
        assert!(split.older[1].starts_with("#0 user"), "{:?}", split.older);
        assert!(
            split.older[2].starts_with("#1 assistant"),
            "{:?}",
            split.older
        );
        assert!(!split.older.iter().any(|l| l.contains("after fork")));
        assert_eq!(split.tail, vec!["#auth:0 user x thread one needle"]);
    }

    #[test]
    fn main_reads_a_thread_by_name() {
        let (_root, main, _dir) = forked();
        let memory = Memory::open(&main).unwrap();
        let zoomed = call(&memory, "zoom", json!({"id": "auth:0"})).unwrap();
        assert!(zoomed.starts_with("#auth:0 user"), "{zoomed}");
        assert!(call(&memory, "zoom", json!({"id": "2"})).is_ok());
        assert_eq!(
            call(&memory, "zoom", json!({"id": "nope:0"})).unwrap_err(),
            "no thread nope; `ralph thread ls` lists them."
        );
    }

    #[test]
    fn a_thread_loads_the_main_config() {
        let (root, main, dir) = forked();
        fs::write(
            main.join("config.toml"),
            "[chat]\nrecent_history_tokens = 1234\n",
        )
        .unwrap();
        let global = root.path().join("global.toml");
        let loaded = load_for_chat(&global, &config_root(&dir).join("config.toml"));
        assert_eq!(loaded.config.chat.recent_history_tokens, Some(1234));
    }

    #[test]
    fn ls_lists_threads_and_drop_refuses_an_open_one() {
        let (_root, main, dir) = forked();
        ensure(&main, &name("b-side")).unwrap();
        let rows = list(&main).unwrap();
        let names: Vec<&str> = rows
            .lines()
            .filter_map(|l| l.split_whitespace().next())
            .collect();
        assert_eq!(names, vec!["auth", "b-side"], "{rows}");
        assert!(rows.contains("forked at #2"), "{rows}");
        assert!(drop_thread(&main, &name("auth"), &["w1".to_string()])
            .unwrap_err()
            .contains("running workers (w1)"));
        assert!(dir.join(THREAD_FILE).is_file());
        let held = Store::new(dir.clone()).claim_chat().unwrap().unwrap();
        assert!(drop_thread(&main, &name("auth"), &[])
            .unwrap_err()
            .contains("in use"));
        held.unlock().unwrap();
        std::mem::drop(held);
        drop_thread(&main, &name("auth"), &[]).unwrap();
        assert!(!dir.exists());
        assert!(drop_thread(&main, &name("auth"), &[]).is_err());
        assert_eq!(list(&main).unwrap().lines().count(), 1);
    }

    fn digest(text: &'static str) -> impl FnMut(&str) -> Result<String, String> {
        move |_| Ok(text.to_string())
    }

    #[test]
    fn merge_posts_a_digest_to_main_and_freezes_the_thread() {
        let (_root, main, dir) = forked();
        let mut asked = Vec::new();
        let printed = merge_thread(&main, &name("auth"), &[], &mut |prompt: &str| {
            asked.push(prompt.to_string());
            Ok("Picked a needle. #auth:0".to_string())
        })
        .unwrap();
        assert_eq!(printed, "[thread auth · merged]\nPicked a needle. #auth:0");
        assert!(asked[0].contains("forked from main at #2"), "{}", asked[0]);
        assert!(asked[0].contains("\n#auth:0 user"), "{}", asked[0]);
        let merged = read_meta(&dir).unwrap().merged.unwrap();
        assert_eq!(merged.digest, "Picked a needle. #auth:0");
        let inbox = Inbox::new(main.join(INBOX_DIR)).peek().unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].worker, "thread:auth");
        assert_eq!(inbox[0].at, merged.at);
        let rows = list(&main).unwrap();
        assert!(rows.contains(&format!("  merged {}", merged.at)), "{rows}");
        let frozen = "thread auth is merged into main and stays read-only";
        let again = merge_thread(&main, &name("auth"), &[], &mut digest("x #auth:0"));
        assert!(again.unwrap_err().starts_with(frozen));
        assert!(drop_thread(&main, &name("auth"), &[])
            .unwrap_err()
            .starts_with(frozen));
        assert!(dir.join(THREAD_FILE).is_file());
        assert_eq!(Inbox::new(main.join(INBOX_DIR)).peek().unwrap().len(), 1);
    }

    #[test]
    fn a_refused_merge_changes_nothing() {
        let (_root, main, dir) = forked();
        let auth = name("auth");
        let rejected = merge_thread(&main, &auth, &[], &mut digest("no cites"));
        assert!(rejected.unwrap_err().contains("rejected 3 times"));
        let busy = merge_thread(&main, &auth, &["w1".to_string()], &mut digest("x #auth:0"));
        assert!(busy.unwrap_err().contains("running workers (w1)"));
        let held = Store::new(dir.clone()).claim_chat().unwrap().unwrap();
        let open = merge_thread(&main, &auth, &[], &mut digest("x #auth:0"));
        assert!(open.unwrap_err().contains("in use"));
        held.unlock().unwrap();
        std::mem::drop(held);
        let unknown = merge_thread(&main, &name("nope"), &[], &mut digest("x #nope:0"));
        assert!(unknown.unwrap_err().starts_with("no thread nope"));
        assert_eq!(read_meta(&dir).unwrap().merged, None);
        let inbox = Inbox::new(main.join(INBOX_DIR)).peek().unwrap_or_default();
        assert!(inbox.is_empty());
    }
}
