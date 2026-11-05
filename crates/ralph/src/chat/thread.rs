use crate::chat::claim::ChatClaim;
use crate::chat::memory::Store;
use crate::cli::{ThreadArgs, ThreadCommand, NAP_TIMEOUT_SECS};
use crate::private_file::check_path;
use crate::schedule_store::ScheduleStore;
use crate::watch_store::WatchStore;
use crate::worker_mcp::Inbox;
use crate::workers::{canonical_store, running_workers};
use crate::{run_curator, Curator};
use ralph_core::chat::VIEW_LINES;
use ralph_core::chat_memory::{view, CallSite, Entry, MemoryCall, Tree, ViewSplit};
use ralph_core::chat_thread::{
    digest_with, drop_message, fork_check, fork_launch, guard_check, label_item, ls_counts,
    merge_prompt, merged_check, merged_report, open_check, pull_message, rebase_block,
    rebased_report, revive_check, thread_split, ForkCommand, ForkLaunch, ForkSpawn, Guarded, Live,
    Merged, Parked, Scope, ThreadMeta, ThreadName, FORK_IN_THREAD, MAIN_PREFIX_LINES, REBASE_LINES,
    THREADS_DIR, THREAD_FILE,
};
use ralph_core::schedule_request::SCHEDULE_DIR;
use ralph_core::watch_store::WATCHES_DIR;
use ralph_core::worker::{chat_message, Report, INBOX_DIR};
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
            let rows = list(&main, &|name| running_ids(&main, name).len())?;
            println!("{}", if rows.is_empty() { "no threads" } else { &rows });
        }
        ThreadCommand::Drop { name } => {
            println!("{}", drop_thread(&main, &name, &running_ids(&main, &name))?);
        }
        ThreadCommand::Merge { name, rebase: true } => {
            let message = rebase_thread(&main, &name, &running_ids(&main, &name))?;
            println!("rebased thread {name} into main's inbox\n{message}");
        }
        ThreadCommand::Merge {
            name,
            rebase: false,
        } => {
            let dir = thread_dir(&main, &name);
            let curator = Curator {
                kind: CallSite::Merge,
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
        ThreadCommand::Park { name } => {
            park_thread(&main, &name, &running_ids(&main, &name))?;
            println!("parked thread {name}; `ralph thread revive {name}` opens it again");
        }
        ThreadCommand::Revive { name } => {
            revive_thread(&main, &name)?;
            println!("revived thread {name}");
        }
        ThreadCommand::Pull { name } => println!("{}", pull_thread(&main, &name)?),
    }
    Ok(())
}

fn running_ids(main: &Path, name: &ThreadName) -> Vec<String> {
    running_workers(&canonical_store(thread_dir(main, name)))
        .into_iter()
        .map(|worker| worker.id)
        .collect()
}

pub(super) fn read_meta(dir: &Path) -> io::Result<ThreadMeta> {
    let text = fs::read_to_string(dir.join(THREAD_FILE))?;
    serde_json::from_str(&text).map_err(io::Error::other)
}

pub fn config_root(store: &Path) -> PathBuf {
    fork_of(store).map_or_else(|| store.to_path_buf(), |(main, _)| main)
}

pub fn store_config_path(written: &Path) -> PathBuf {
    config_root(written).join("config.toml")
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
        parked: None,
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
    merge_with(main, name, running, |meta, entries, tree, at| {
        let lines: Vec<String> = view(entries, tree, VIEW_LINES)
            .iter()
            .map(|line| label_item(line, name))
            .collect();
        let digest = digest_with(&merge_prompt(name, meta.base, &lines), name, curate)
            .map_err(|e| format!("thread {name} was not merged: {e}"))?;
        Ok(merged_report(name, &digest, at))
    })
}

pub fn rebase_thread(main: &Path, name: &ThreadName, running: &[String]) -> Result<String, String> {
    merge_with(main, name, running, |meta, entries, tree, at| {
        let block = rebase_block(name, meta.base, &view(entries, tree, REBASE_LINES))?;
        Ok(rebased_report(name, &block, at))
    })
}

fn merge_with(
    main: &Path,
    name: &ThreadName,
    running: &[String],
    report: impl FnOnce(&ThreadMeta, &[Entry], &Tree, &str) -> Result<Report, String>,
) -> Result<String, String> {
    let dir = thread_dir(main, name);
    let meta = known_meta(&dir, name)?;
    merged_check(name, &meta)?;
    guard_check(name, Guarded::Merge, live(&dir, running))?;
    let _claim = claim_closed(&dir, name)?;
    let failed = |e: io::Error| format!("could not merge thread {name}: {e}");
    let (entries, tree) = Store::new(dir.clone()).load().map_err(failed)?;
    let at = chrono::Utc::now().to_rfc3339();
    let report = report(&meta, &entries, &tree, &at)?;
    let merged = ThreadMeta {
        merged: Some(Merged {
            at,
            digest: report.text.clone(),
        }),
        parked: None,
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

fn live<'a>(dir: &Path, running: &'a [String]) -> Live<'a> {
    let records = WatchStore::new(dir.join(WATCHES_DIR)).load_all().len()
        + ScheduleStore::new(dir.join(SCHEDULE_DIR)).load_all().len();
    Live {
        workers: running,
        records,
    }
}

fn known_meta(dir: &Path, name: &ThreadName) -> Result<ThreadMeta, String> {
    if !dir.join(THREAD_FILE).is_file() {
        return Err(format!("no thread {name}; `ralph thread ls` lists them."));
    }
    read_meta(dir).map_err(|e| format!("thread {name}: {e}"))
}

pub(super) fn claim_closed(dir: &Path, name: &ThreadName) -> Result<ChatClaim, String> {
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

fn fork_ready(store: &Path, name: &ThreadName) -> Result<(), String> {
    if fork_of(store).is_some() {
        return Err(FORK_IN_THREAD.to_string());
    }
    let dir = thread_dir(store, name);
    if !dir.join(THREAD_FILE).is_file() {
        return Ok(());
    }
    let meta = read_meta(&dir).map_err(|e| format!("thread {name}: {e}"))?;
    open_check(name, &meta)?;
    claim_closed(&dir, name).map(drop)
}

pub fn fork(store: &Path, command: ForkCommand) -> String {
    let name = match command {
        ForkCommand::Open(name) => name,
        ForkCommand::Usage(usage) => return usage,
    };
    if let Err(refused) = fork_ready(store, &name) {
        return refused;
    }
    let (exe, cwd) = match (std::env::current_exe(), std::env::current_dir()) {
        (Ok(exe), Ok(cwd)) => (exe, cwd),
        (Err(e), _) | (_, Err(e)) => return format!("could not fork thread {name}: {e}"),
    };
    let (exe, main, cwd) = (
        exe.to_string_lossy(),
        store.to_string_lossy(),
        cwd.to_string_lossy(),
    );
    let block = std::env::var("REX_BLOCK").ok();
    let config_dir = std::env::var("RALPH_CONFIG_DIR").ok();
    let spawn = ForkSpawn {
        name: &name,
        exe: &exe,
        main: &main,
        cwd: &cwd,
        block: block.as_deref(),
        config_dir: config_dir.as_deref(),
    };
    match fork_launch(spawn) {
        ForkLaunch::Manual(text) => text,
        ForkLaunch::Rex(argv) => match std::process::Command::new("rex").args(&argv).output() {
            Ok(out) if out.status.success() => format!("opened thread {name} in a new Rex block"),
            Ok(out) => format!(
                "rex split failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ),
            Err(e) => format!("could not run rex: {e}"),
        },
    }
}

pub fn list(main: &Path, workers: &dyn Fn(&ThreadName) -> usize) -> io::Result<String> {
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
        let messages = Store::new(dir.clone()).log_len()?;
        let unread = Inbox::new(dir.join(INBOX_DIR)).unread_count()?;
        let counts = ls_counts(workers(&name), unread);
        let merged = meta
            .merged
            .map(|merged| format!("  merged {}", merged.at))
            .unwrap_or_default();
        let parked = meta
            .parked
            .map(|parked| format!("  parked {}", parked.at))
            .unwrap_or_default();
        rows.push(format!(
            "{name}  forked at #{}  {messages} messages  {}{merged}{parked}{counts}",
            meta.base, meta.created
        ));
    }
    Ok(rows.join("\n"))
}

pub fn drop_thread(main: &Path, name: &ThreadName, running: &[String]) -> Result<String, String> {
    let dir = thread_dir(main, name);
    merged_check(name, &known_meta(&dir, name)?)?;
    let live = live(&dir, running);
    guard_check(name, Guarded::Drop, live)?;
    let _claim = claim_closed(&dir, name)?;
    fs::remove_dir_all(&dir).map_err(|e| format!("could not drop thread {name}: {e}"))?;
    Ok(drop_message(name, live.records))
}

pub fn park_thread(main: &Path, name: &ThreadName, running: &[String]) -> Result<(), String> {
    let dir = thread_dir(main, name);
    let meta = known_meta(&dir, name)?;
    open_check(name, &meta)?;
    guard_check(name, Guarded::Park, live(&dir, running))?;
    let _claim = claim_closed(&dir, name)?;
    let parked = Some(Parked {
        at: chrono::Utc::now().to_rfc3339(),
    });
    write_meta(&dir, &ThreadMeta { parked, ..meta })
        .map_err(|e| format!("could not park thread {name}: {e}"))
}

pub fn revive_thread(main: &Path, name: &ThreadName) -> Result<(), String> {
    let dir = thread_dir(main, name);
    let meta = known_meta(&dir, name)?;
    revive_check(name, &meta)?;
    let _claim = claim_closed(&dir, name)?;
    write_meta(
        &dir,
        &ThreadMeta {
            parked: None,
            ..meta
        },
    )
    .map_err(|e| format!("could not revive thread {name}: {e}"))
}

pub fn pull_thread(main: &Path, name: &ThreadName) -> Result<String, String> {
    let dir = thread_dir(main, name);
    let meta = known_meta(&dir, name)?;
    merged_check(name, &meta)?;
    let _claim = claim_closed(&dir, name)?;
    let failed = |e: io::Error| format!("could not pull thread {name}: {e}");
    let end = Store::new(main.to_path_buf()).log_len().map_err(failed)?;
    let message = pull_message(name, meta.base, end);
    if end != meta.base {
        write_meta(&dir, &ThreadMeta { base: end, ..meta }).map_err(failed)?;
    }
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_for_chat;
    use ralph_core::chat_memory::{parse_tool_call, Role};
    use ralph_core::inbox::Selection;
    use ralph_core::schedule::{Task, TaskKind, TaskStatus};
    use ralph_core::watch::WatchKind;
    use ralph_core::watch_store::SavedWatch;
    use serde_json::json;
    use std::fs;

    fn name(s: &str) -> ThreadName {
        ThreadName::parse(s).unwrap()
    }

    fn call(memory: &Memory, tool: &str, args: serde_json::Value) -> Result<String, String> {
        memory.call(&parse_tool_call(tool, &args).unwrap())
    }

    #[cfg(unix)]
    #[test]
    fn store_config_of_a_plain_store_linked_to_a_thread_folder_stays_its_own() {
        let root = tempfile::tempdir().unwrap();
        let base = root.path();
        let x_foo = base.join("x").join("threads").join("foo");
        fs::create_dir_all(&x_foo).unwrap();
        let plain = base.join("plain");
        std::os::unix::fs::symlink(&x_foo, &plain).unwrap();
        assert_eq!(store_config_path(&plain), plain.join("config.toml"));
    }

    #[cfg(unix)]
    #[test]
    fn store_config_of_a_linked_thread_folder_is_mains() {
        let root = tempfile::tempdir().unwrap();
        let base = root.path();
        let elsewhere = base.join("elsewhere");
        fs::create_dir_all(&elsewhere).unwrap();
        let m_threads = base.join("m").join("threads");
        fs::create_dir_all(&m_threads).unwrap();
        let linked = m_threads.join("foo");
        std::os::unix::fs::symlink(&elsewhere, &linked).unwrap();
        assert_eq!(
            store_config_path(&linked),
            base.join("m").join("config.toml")
        );
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
        let rows = list(&main, &|_| 0).unwrap();
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
        std::mem::drop(held);
        drop_thread(&main, &name("auth"), &[]).unwrap();
        assert!(!dir.exists());
        assert!(drop_thread(&main, &name("auth"), &[]).is_err());
        assert_eq!(list(&main, &|_| 0).unwrap().lines().count(), 1);
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
        let rows = list(&main, &|_| 0).unwrap();
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
        std::mem::drop(held);
        let unknown = merge_thread(&main, &name("nope"), &[], &mut digest("x #nope:0"));
        assert!(unknown.unwrap_err().starts_with("no thread nope"));
        assert_eq!(read_meta(&dir).unwrap().merged, None);
        let inbox = Inbox::new(main.join(INBOX_DIR)).peek().unwrap_or_default();
        assert!(inbox.is_empty());
    }

    #[test]
    fn park_freezes_a_thread_until_revive() {
        let (_root, main, dir) = forked();
        let auth = name("auth");
        park_thread(&main, &auth, &[]).unwrap();
        let parked = read_meta(&dir).unwrap().parked.unwrap();
        let rows = list(&main, &|_| 0).unwrap();
        assert!(rows.ends_with(&format!("  parked {}", parked.at)), "{rows}");
        assert_eq!(
            park_thread(&main, &auth, &[]).unwrap_err(),
            "thread auth is parked; `ralph thread revive auth` opens it again."
        );
        revive_thread(&main, &auth).unwrap();
        assert_eq!(read_meta(&dir).unwrap().parked, None);
        assert_eq!(
            revive_thread(&main, &auth).unwrap_err(),
            "thread auth is not parked."
        );
    }

    #[test]
    fn a_refused_park_changes_nothing() {
        let (_root, main, dir) = forked();
        let auth = name("auth");
        let busy = park_thread(&main, &auth, &["w1".to_string()]);
        assert!(busy.unwrap_err().contains("running workers (w1)"));
        let held = Store::new(dir.clone()).claim_chat().unwrap().unwrap();
        assert!(park_thread(&main, &auth, &[])
            .unwrap_err()
            .contains("in use"));
        std::mem::drop(held);
        let unknown = park_thread(&main, &name("nope"), &[]);
        assert!(unknown.unwrap_err().starts_with("no thread nope"));
        assert!(revive_thread(&main, &name("nope"))
            .unwrap_err()
            .starts_with("no thread nope"));
        merge_thread(&main, &auth, &[], &mut digest("x #auth:0")).unwrap();
        let frozen = "thread auth is merged into main and stays read-only";
        assert!(park_thread(&main, &auth, &[])
            .unwrap_err()
            .starts_with(frozen));
        assert!(revive_thread(&main, &auth).unwrap_err().starts_with(frozen));
        assert_eq!(read_meta(&dir).unwrap().parked, None);
    }

    #[test]
    fn merging_a_parked_thread_unparks_it_and_drop_still_works() {
        let (_root, main, dir) = forked();
        let auth = name("auth");
        let parked = ThreadMeta {
            parked: Some(Parked {
                at: "p".to_string(),
            }),
            ..read_meta(&dir).unwrap()
        };
        write_meta(&dir, &parked).unwrap();
        merge_thread(&main, &auth, &[], &mut digest("x #auth:0")).unwrap();
        let meta = read_meta(&dir).unwrap();
        assert!(meta.merged.is_some());
        assert_eq!(meta.parked, None);
        ensure(&main, &name("spare")).unwrap();
        park_thread(&main, &name("spare"), &[]).unwrap();
        drop_thread(&main, &name("spare"), &[]).unwrap();
        assert!(!thread_dir(&main, &name("spare")).exists());
    }

    #[test]
    fn fork_refuses_open_parked_merged_and_from_inside_a_thread() {
        let (_root, main, dir) = forked();
        let auth = name("auth");
        assert_eq!(fork_ready(&main, &name("fresh")), Ok(()));
        assert!(!thread_dir(&main, &name("fresh")).exists());
        assert_eq!(fork_ready(&main, &auth), Ok(()));
        assert_eq!(fork_ready(&main, &auth), Ok(()));
        let held = Store::new(dir.clone()).claim_chat().unwrap().unwrap();
        let open = fork_ready(&main, &auth).unwrap_err();
        assert!(open.starts_with("thread auth is in use"), "{open}");
        drop(held);
        assert_eq!(
            fork_ready(&dir, &name("other")).unwrap_err(),
            FORK_IN_THREAD
        );
        park_thread(&main, &auth, &[]).unwrap();
        let parked = fork_ready(&main, &auth).unwrap_err();
        assert!(parked.contains("is parked"), "{parked}");
        merge_thread(&main, &auth, &[], &mut digest("x #auth:0")).unwrap();
        let merged = fork_ready(&main, &auth).unwrap_err();
        assert!(merged.contains("merged"), "{merged}");
    }

    fn save_records(dir: &Path) {
        WatchStore::new(dir.join(WATCHES_DIR))
            .save(&SavedWatch {
                id: "watch:w1".to_string(),
                command: "sleep 600".to_string(),
                follow_up: None,
                kind: WatchKind::Persistent,
                timeout_secs: None,
                started_at: chrono::Utc::now(),
                timeout_deadline: None,
                wake_deadline: None,
            })
            .unwrap();
        ScheduleStore::new(dir.join(SCHEDULE_DIR))
            .save(&Task {
                id: "t1".to_string(),
                kind: TaskKind::Command {
                    command: "true".to_string(),
                },
                follow_up: None,
                at: chrono::Utc::now(),
                timeout_secs: None,
                status: TaskStatus::Pending,
                late: false,
                deadline: None,
            })
            .unwrap();
    }

    #[test]
    fn rebase_posts_the_block_to_main_and_freezes_the_thread_without_a_curator() {
        let (_root, main, dir) = forked();
        let auth = name("auth");
        let printed = rebase_thread(&main, &auth, &[]).unwrap();
        let head = "[thread auth · rebased]\nThread auth, forked from main at #2, rebased onto main as its memory view:\n#auth:0 user";
        assert!(printed.starts_with(head), "{printed}");
        let merged = read_meta(&dir).unwrap().merged.unwrap();
        assert!(printed.ends_with(&merged.digest), "{printed}");
        let inbox = Inbox::new(main.join(INBOX_DIR)).peek().unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(
            (inbox[0].worker.as_str(), inbox[0].status.as_str()),
            ("thread:auth", "rebased")
        );
        assert_eq!((&inbox[0].text, &inbox[0].at), (&merged.digest, &merged.at));
        let again = rebase_thread(&main, &auth, &[]).unwrap_err();
        assert!(
            again.starts_with("thread auth is merged into main"),
            "{again}"
        );
        assert_eq!(Inbox::new(main.join(INBOX_DIR)).peek().unwrap().len(), 1);
    }

    #[test]
    fn rebase_refuses_like_merge_before_writing() {
        let (_root, main, dir) = forked();
        let auth = name("auth");
        let busy = rebase_thread(&main, &auth, &["w1".to_string()]).unwrap_err();
        assert!(busy.contains("running workers (w1)"), "{busy}");
        let held = Store::new(dir.clone()).claim_chat().unwrap().unwrap();
        let open = rebase_thread(&main, &auth, &[]).unwrap_err();
        assert!(open.contains("in use"), "{open}");
        std::mem::drop(held);
        save_records(&dir);
        let saved = rebase_thread(&main, &auth, &[]).unwrap_err();
        assert!(saved.starts_with("thread auth has 2 saved"), "{saved}");
        let unknown = rebase_thread(&main, &name("nope"), &[]).unwrap_err();
        assert!(unknown.starts_with("no thread nope"), "{unknown}");
        ensure(&main, &name("empty")).unwrap();
        assert_eq!(
            rebase_thread(&main, &name("empty"), &[]).unwrap_err(),
            "thread empty has no messages to rebase."
        );
        assert_eq!(
            read_meta(&thread_dir(&main, &name("empty")))
                .unwrap()
                .merged,
            None
        );
        assert_eq!(read_meta(&dir).unwrap().merged, None);
        let inbox = Inbox::new(main.join(INBOX_DIR)).peek().unwrap_or_default();
        assert!(inbox.is_empty());
    }

    #[test]
    fn merge_refuses_saved_records_before_asking_the_curator() {
        let (_root, main, dir) = forked();
        let auth = name("auth");
        save_records(&dir);
        let mut asked = 0;
        let refused = merge_thread(&main, &auth, &[], &mut |_: &str| {
            asked += 1;
            Ok("x #auth:0".to_string())
        });
        assert_eq!(
            refused.unwrap_err(),
            "thread auth has 2 saved watches or schedules; stop them from thread auth's chat before merging."
        );
        assert_eq!(asked, 0);
        assert_eq!(read_meta(&dir).unwrap().merged, None);
        assert!(Inbox::new(main.join(INBOX_DIR))
            .peek()
            .unwrap_or_default()
            .is_empty());
    }

    #[test]
    fn park_keeps_saved_records_and_drop_reports_them() {
        let (_root, main, dir) = forked();
        let auth = name("auth");
        save_records(&dir);
        park_thread(&main, &auth, &[]).unwrap();
        revive_thread(&main, &auth).unwrap();
        assert_eq!(live(&dir, &[]).records, 2);
        assert_eq!(
            drop_thread(&main, &auth, &[]).unwrap(),
            "dropped thread auth and its 2 saved watches or schedules"
        );
        assert!(!dir.exists());
    }

    #[test]
    fn ls_counts_workers_and_unread_without_touching_the_inbox() {
        let (_root, main, dir) = forked();
        let auth = name("auth");
        let inbox = Inbox::new(dir.join(INBOX_DIR));
        inbox.post(&merged_report(&auth, "a", "t1"), 0).unwrap();
        let files = || fs::read_dir(dir.join(INBOX_DIR)).unwrap().count();
        let rows = list(&main, &|_| 2).unwrap();
        assert!(rows.ends_with("  2 workers  1 unread"), "{rows}");
        assert_eq!(files(), 1);
        assert!(!dir.join("inbox.json").exists());
        assert_eq!(inbox.peek().unwrap().len(), 1);
        inbox.post(&merged_report(&auth, "b", "t2"), 1).unwrap();
        let rows = list(&main, &|_| 1).unwrap();
        assert!(rows.ends_with("  1 worker  2 unread"), "{rows}");
        assert_eq!(inbox.drain(Selection::All).unwrap().len(), 2);
        let rows = list(&main, &|_| 0).unwrap();
        assert!(
            !rows.contains("worker") && !rows.contains("unread"),
            "{rows}"
        );
    }

    #[test]
    fn pull_moves_the_fork_point_to_main_end() {
        let (_root, main, dir) = forked();
        let auth = name("auth");
        let before = Memory::open(&dir).unwrap();
        assert!(call(&before, "zoom", json!({"id": "2"})).is_err());
        assert_eq!(
            pull_thread(&main, &auth).unwrap(),
            "pulled thread auth: fork point #2 -> #3, 1 new main message."
        );
        assert_eq!(read_meta(&dir).unwrap().base, 3);
        let after = Memory::open(&dir).unwrap();
        let zoomed = call(&after, "zoom", json!({"id": "2"})).unwrap();
        assert!(zoomed.contains("main after fork needle"), "{zoomed}");
        assert_eq!(
            pull_thread(&main, &auth).unwrap(),
            "thread auth is already at main's end (#3)."
        );
    }

    #[test]
    fn a_refused_pull_changes_nothing() {
        let (_root, main, dir) = forked();
        let auth = name("auth");
        let held = Store::new(dir.clone()).claim_chat().unwrap().unwrap();
        assert!(pull_thread(&main, &auth).unwrap_err().contains("in use"));
        std::mem::drop(held);
        assert!(pull_thread(&main, &name("nope"))
            .unwrap_err()
            .starts_with("no thread nope"));
        merge_thread(&main, &auth, &[], &mut digest("x #auth:0")).unwrap();
        assert!(pull_thread(&main, &auth)
            .unwrap_err()
            .starts_with("thread auth is merged into main and stays read-only"));
        assert_eq!(read_meta(&dir).unwrap().base, 2);
    }
}
