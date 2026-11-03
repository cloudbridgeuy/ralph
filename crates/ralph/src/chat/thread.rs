use crate::chat::memory::Store;
use crate::cli::{ThreadArgs, ThreadCommand};
use crate::private_file::check_path;
use crate::workers::{canonical_store, running_workers};
use ralph_core::chat_memory::{view, MemoryCall, ViewSplit};
use ralph_core::chat_thread::{
    fork_check, idle_check, thread_split, Scope, ThreadMeta, ThreadName, MAIN_PREFIX_LINES,
    THREADS_DIR, THREAD_FILE,
};
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
            let running: Vec<String> = running_workers(&canonical_store(thread_dir(&main, &name)))
                .into_iter()
                .map(|worker| worker.id)
                .collect();
            drop_thread(&main, &name, &running)?;
            println!("dropped thread {name}");
        }
    }
    Ok(())
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
    };
    fs::create_dir_all(&dir)?;
    let path = dir.join(THREAD_FILE);
    check_path(&path)?;
    fs::write(
        path,
        serde_json::to_string(&meta).map_err(io::Error::other)?,
    )?;
    Ok(meta)
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
        rows.push(format!(
            "{name}  forked at #{}  {messages} messages  {}",
            meta.base, meta.created
        ));
    }
    Ok(rows.join("\n"))
}

pub fn drop_thread(main: &Path, name: &ThreadName, running: &[String]) -> Result<(), String> {
    let dir = thread_dir(main, name);
    if !dir.join(THREAD_FILE).is_file() {
        return Err(format!("no thread {name}; `ralph thread ls` lists them."));
    }
    idle_check(name, running)?;
    let failed = |e: io::Error| format!("could not drop thread {name}: {e}");
    let _claim = Store::new(dir.clone())
        .claim_chat()
        .map_err(failed)?
        .map_err(|holder| {
            let by = holder
                .map(|pid| format!(" by pid {pid}"))
                .unwrap_or_default();
            format!("thread {name} is in use{by}; close its chat first.")
        })?;
    fs::remove_dir_all(&dir).map_err(failed)
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
}
