use crate::claude_cleanup::cleanup_worktree_leftovers;
use crate::config::AppConfig;
use crate::cursor_cleanup::cleanup_cursor_leftovers;
use crate::cursor_owner::{caller_owns_cursor, owned_cursor_ids, sweep_owner, write_file};
use crate::opencode_worker::{entry_ids, running_opencode_sessions};
use crate::warn::warn_if_err;
use ralph_core::chat_status::{store_workers, RunningWorker};
use ralph_core::claude_leftovers::cleans_claude_leftovers;
use ralph_core::cursor_leftovers::cleans_cursor_leftovers;
use ralph_core::worker::{
    base_branch, base_config_key, blocked_path, branch_name, close_handoff, close_plan,
    interrupt_reply, marker_paths, parse_interrupt_result, project_slug, prompt_entry_name,
    resolve_worktree_dir, sent_path, session_name, session_owner, tagged_sessions,
    worktree_for_branch, worktree_path, CloseState, CloseStep, CursorFiles, Handoff, OpencodeFiles,
    Owner, Sweep, TaggedSession, WorktreeDir, HANDOFF_POLL, HANDOFF_TIMEOUT, OPENCODE_CONFIG_FILE,
    OPENCODE_OWNER_DIR, SESSION_FORMAT, WORKTREE_DIR_ENV,
};
use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{self, Command, Output, Stdio};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

mod spawn;

pub struct Workers {
    store_dir: PathBuf,
    exe: PathBuf,
    permission_mode: String,
    worktree_dir: Option<String>,
}

impl Workers {
    pub fn new(
        store_dir: PathBuf,
        exe: PathBuf,
        permission_mode: String,
        worktree_dir: Option<String>,
    ) -> Self {
        Self {
            store_dir: canonical_store(store_dir),
            exe,
            permission_mode,
            worktree_dir,
        }
    }

    fn mark_sent(&self, id: &str, sent: io::Result<String>) -> io::Result<String> {
        if sent.is_ok() {
            warn_if_err(
                write_file(&sent_path(&self.store_dir, id), ""),
                format!("marking worker {id} as sent"),
            );
        }
        sent
    }

    pub fn reply(&self, id: &str, text: &str, interrupt: bool) -> io::Result<String> {
        let delivered = self.deliver(id, text, interrupt);
        self.mark_sent(id, delivered)
    }

    fn deliver(&self, id: &str, text: &str, interrupt: bool) -> io::Result<String> {
        let files = CursorFiles::new(&self.store_dir, id);
        if files.owner.exists() {
            if interrupt {
                let entry = self.queue_entry(&files.interrupt, text)?;
                return Ok(interrupt_reply(
                    id,
                    self.await_interrupt_result(&files, &entry),
                ));
            }
            if self.awaits_permission(id) {
                return self.answer_permission(id, text);
            }
            return self.queue_prompt(&files.prompt, id, text, "Sent to");
        }
        if self.opencode_http(id) {
            return self.deliver_opencode(id, text, interrupt, "Sent to");
        }
        if interrupt {
            return Err(io::Error::other(
                "interrupt needs a Cursor or opencode worker",
            ));
        }
        let session = match self.owner(&project_slug(&repo_root()?), id) {
            Owner::Mine(session) => session.name,
            Owner::Missing => return Err(io::Error::other(format!("worker {id} is not running"))),
            Owner::Foreign(session) => {
                return Err(io::Error::other(format!(
                    "worker {id} belongs to another chat (store {})",
                    session.store
                )))
            }
        };
        let target = format!("={session}:");
        let mut load = Command::new("tmux")
            .args(["load-buffer", "-b", &session, "-"])
            .stdin(Stdio::piped())
            .spawn()?;
        if let Some(mut stdin) = load.stdin.take() {
            stdin.write_all(text.as_bytes())?;
        }
        check(load.wait_with_output()?)?;
        run(Command::new("tmux").args([
            "paste-buffer",
            "-p",
            "-d",
            "-b",
            &session,
            "-t",
            &target,
        ]))?;
        run(Command::new("tmux").args(["send-keys", "-t", &target, "Enter"]))?;
        Ok(format!("Sent to worker {id}."))
    }

    pub fn nudge(&self, id: &str, text: &str) -> io::Result<String> {
        let files = CursorFiles::new(&self.store_dir, id);
        if files.owner.exists() {
            let queued = self.queue_prompt(&files.prompt, id, text, "Nudged");
            return self.mark_sent(id, queued);
        }
        if self.opencode_http(id) {
            let queued = self.deliver_opencode(id, text, false, "Nudged");
            return self.mark_sent(id, queued);
        }
        self.reply(id, text, false)
    }

    fn opencode_http(&self, id: &str) -> bool {
        let files = OpencodeFiles::new(&self.store_dir, id);
        files.owner.exists() || files.session.exists()
    }

    fn deliver_opencode(
        &self,
        id: &str,
        text: &str,
        interrupt: bool,
        verb: &str,
    ) -> io::Result<String> {
        let files = OpencodeFiles::new(&self.store_dir, id);
        if interrupt {
            return self.queue_prompt(&files.interrupt, id, text, "Interrupted");
        }
        if self.awaits_permission(id) {
            return self.answer_permission(id, text);
        }
        self.queue_prompt(&files.prompt, id, text, verb)
    }

    fn await_interrupt_result(&self, files: &CursorFiles, entry: &str) -> Option<bool> {
        let result = files.interrupt_result.join(entry);
        let started = Instant::now();
        loop {
            if let Ok(text) = fs::read_to_string(&result) {
                fs::remove_file(&result).ok();
                return parse_interrupt_result(&text);
            }
            if started.elapsed() >= HANDOFF_TIMEOUT {
                return None;
            }
            thread::sleep(HANDOFF_POLL);
        }
    }

    fn queue_prompt(&self, dir: &Path, id: &str, text: &str, verb: &str) -> io::Result<String> {
        self.queue_entry(dir, text)?;
        Ok(format!("{verb} worker {id}."))
    }

    fn queue_entry(&self, dir: &Path, text: &str) -> io::Result<String> {
        let entry = prompt_entry_name(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| {
                    u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX)
                }),
            process::id(),
        );
        let pending = dir.join(format!(".{entry}"));
        write_file(&pending, text)?;
        fs::rename(&pending, dir.join(&entry))?;
        Ok(entry)
    }

    pub fn close(&self, id: &str, abandon: bool, force: bool) -> io::Result<String> {
        let repo = repo_root()?;
        self.refuse_busy_cursor(id, true)?;
        let owner = self.owner(&project_slug(&repo), id);
        let config = AppConfig::load().map_err(|e| e.to_string());
        let (worktree, worktree_dir) =
            locate_worktree(&repo, id, self.worktree_dir.as_deref(), config.as_ref())?;
        let branch = branch_name(id);
        let dirty_worktree = if worktree.join(".git").exists() {
            Some(
                !run(Command::new("git")
                    .arg("-C")
                    .arg(&worktree)
                    .args(["status", "--porcelain"]))?
                .stdout
                .is_empty(),
            )
        } else {
            None
        };
        let base = base_branch(
            config
                .as_ref()
                .map(|config| config.chat.worker_base.as_deref())
                .map_err(String::as_str),
            git_value(&repo, &["config", "--get", &base_config_key(id)])?.as_deref(),
            git_value(&repo, &["symbolic-ref", "-q", "refs/remotes/origin/HEAD"])?.as_deref(),
        );
        let unmerged_commits = if branch_exists(&repo, &branch)? {
            let count = stdout_line(run(Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(["rev-list", "--count"])
                .arg(format!("{}..{branch}", base.branch)))?);
            Some(count.parse().map_err(io::Error::other)?)
        } else {
            None
        };
        let opencode = matches!(&owner, Owner::Mine(session) | Owner::Foreign(session) if session.opencode)
            || worktree.join(OPENCODE_CONFIG_FILE).exists();
        let state = CloseState {
            id: id.to_string(),
            owner,
            dirty_worktree,
            base: base.branch.clone(),
            unmerged_commits,
            opencode,
            markers: marker_paths(&self.store_dir, id)
                .iter()
                .any(|marker| marker.exists()),
            cursor: CursorFiles::new(&self.store_dir, id).owner.exists() || self.opencode_http(id),
        };
        let with_warning = |text: String| {
            [&base.warning, &worktree_dir.warning]
                .into_iter()
                .flatten()
                .fold(text, |text, warning| format!("{text} {warning}"))
        };
        let steps = close_plan(&state, abandon, force)
            .map_err(|refusal| io::Error::other(with_warning(refusal)))?;
        let real_worktree = fs::canonicalize(&worktree).unwrap_or_else(|_| worktree.clone());
        for step in &steps {
            self.close_step(step, &repo, &worktree, id).map_err(|e| {
                io::Error::other(format!("closing worker {id} stopped at {step:?}: {e}"))
            })?;
        }
        let files = CursorFiles::new(&self.store_dir, id);
        clear_marker(&files.session)?;
        clear_marker(&files.interrupt_result)?;
        if steps == [CloseStep::ClearMarkers] {
            return Ok(format!(
                "Closed worker {id}: only its markers were left; removed them."
            ));
        }
        let note = if opencode {
            " Evicted its opencode location."
        } else {
            ""
        };
        let leftovers = if cleans_claude_leftovers(&state, &steps) {
            dirs::home_dir()
                .map(|home| cleanup_worktree_leftovers(&home, &worktree))
                .unwrap_or_default()
        } else if cleans_cursor_leftovers(&state, &steps) {
            let porcelain = run(Command::new("git").arg("-C").arg(&repo).args([
                "worktree",
                "list",
                "--porcelain",
            ]))
            .ok()
            .map(|listing| String::from_utf8_lossy(&listing.stdout).into_owned());
            dirs::home_dir()
                .map(|home| {
                    cleanup_cursor_leftovers(&home, &worktree, &real_worktree, porcelain.as_deref())
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        Ok(leftovers.into_iter().fold(
            with_warning(format!(
                "Closed worker {id}: removed its sessions, worktree and branch {branch}.{note}"
            )),
            |text, warning| format!("{text} {warning}"),
        ))
    }

    fn close_step(
        &self,
        step: &CloseStep,
        repo: &Path,
        worktree: &Path,
        id: &str,
    ) -> io::Result<()> {
        let git = || {
            let mut git = Command::new("git");
            git.arg("-C").arg(repo);
            git
        };
        match step {
            CloseStep::RemoveOpencodeSession => {
                self.terminate_opencode(id)?;
            }
            CloseStep::TerminateCursor => {
                if CursorFiles::new(&self.store_dir, id).owner.exists() {
                    self.terminate_cursor(id)?;
                }
            }
            CloseStep::KillSession(session) => {
                run(Command::new("tmux").args(["kill-session", "-t", &format!("={session}")]))?;
            }
            CloseStep::RemoveWorktree { force } => {
                let mut remove = git();
                remove.args(["worktree", "remove"]);
                if *force {
                    remove.arg("--force");
                }
                run(remove.arg(worktree))?;
            }
            CloseStep::DeleteBranch => {
                run(git().args(["branch", "-D", &branch_name(id)]))?;
            }
            CloseStep::PruneWorktrees => {
                run(git().args(["worktree", "prune"]))?;
            }
            CloseStep::ClearMarkers => {
                for marker in marker_paths(&self.store_dir, id) {
                    clear_marker(&marker)?;
                }
            }
        }
        Ok(())
    }

    pub fn awaits_permission(&self, id: &str) -> bool {
        (CursorFiles::new(&self.store_dir, id).owner.exists() || self.opencode_http(id))
            && blocked_path(&self.store_dir, id).exists()
    }

    fn answer_permission(&self, id: &str, text: &str) -> io::Result<String> {
        let (answer, error) = if self.opencode_http(id) {
            let files = OpencodeFiles::new(&self.store_dir, id);
            (files.answer, files.error)
        } else {
            let files = CursorFiles::new(&self.store_dir, id);
            (files.answer, files.error)
        };
        clear_marker(&error)?;
        write_file(&answer, text)?;
        let started = Instant::now();
        loop {
            match close_handoff(answer.exists(), started.elapsed()) {
                Handoff::Done => break,
                Handoff::TimedOut => {
                    clear_marker(&answer)?;
                    return Err(io::Error::other(
                        "ralph chat did not pick up the permission answer in time",
                    ));
                }
                Handoff::Pending | Handoff::Failed(_) => thread::sleep(HANDOFF_POLL),
            }
        }
        match fs::read_to_string(&error) {
            Ok(message) => {
                clear_marker(&error)?;
                Err(io::Error::other(message))
            }
            Err(_) => Ok(format!("Answered the permission request of worker {id}.")),
        }
    }

    fn terminate_opencode(&self, id: &str) -> io::Result<()> {
        let files = OpencodeFiles::new(&self.store_dir, id);
        clear_marker(&files.error)?;
        write_file(&files.close, "")?;
        let started = Instant::now();
        loop {
            if let Ok(message) = fs::read_to_string(&files.error) {
                clear_marker(&files.error)?;
                return Err(io::Error::other(message));
            }
            match close_handoff(files.owner.exists(), started.elapsed()) {
                Handoff::Done => return Ok(()),
                Handoff::TimedOut => {
                    clear_marker(&files.close)?;
                    return Err(io::Error::other(
                        "ralph chat did not confirm stopping the opencode worker in time",
                    ));
                }
                Handoff::Pending | Handoff::Failed(_) => thread::sleep(HANDOFF_POLL),
            }
        }
    }

    fn refuse_busy_cursor(&self, id: &str, closing: bool) -> io::Result<()> {
        match sweep_owner(&self.store_dir, id) {
            Some(Sweep::Busy) if closing && caller_owns_cursor(&self.store_dir, id) => Ok(()),
            Some(Sweep::Busy) => Err(io::Error::other(format!(
                "worker {id} is a cursor worker owned by another running ralph chat"
            ))),
            _ => Ok(()),
        }
    }

    fn terminate_cursor(&self, id: &str) -> io::Result<()> {
        let files = CursorFiles::new(&self.store_dir, id);
        write_file(&files.close, "")?;
        let started = Instant::now();
        loop {
            match close_handoff(files.owner.exists(), started.elapsed()) {
                Handoff::Done => return Ok(()),
                Handoff::TimedOut => {
                    clear_marker(&files.close)?;
                    return Err(io::Error::other(
                        "ralph chat did not confirm stopping the cursor worker in time",
                    ));
                }
                Handoff::Pending | Handoff::Failed(_) => thread::sleep(HANDOFF_POLL),
            }
        }
    }

    fn owner(&self, project: &str, id: &str) -> Owner {
        if CursorFiles::new(&self.store_dir, id).owner.exists() {
            return Owner::Mine(TaggedSession {
                name: session_name(project, id),
                store: self.store_dir.to_string_lossy().into_owned(),
                opencode: false,
                id: id.to_string(),
            });
        }
        if self.opencode_http(id) {
            return Owner::Mine(TaggedSession {
                name: session_name(project, id),
                store: self.store_dir.to_string_lossy().into_owned(),
                opencode: true,
                id: id.to_string(),
            });
        }
        let sessions: Vec<TaggedSession> = tagged_sessions(&tmux_listing())
            .into_iter()
            .map(|mut session| {
                if !session.store.is_empty() {
                    session.store = canonical_store(PathBuf::from(&session.store))
                        .to_string_lossy()
                        .into_owned();
                }
                session
            })
            .collect();
        session_owner(&sessions, &self.store_dir.to_string_lossy(), project, id)
    }
}

pub fn canonical_store(path: PathBuf) -> PathBuf {
    fs::canonicalize(&path)
        .or_else(|_| std::path::absolute(&path))
        .unwrap_or(path)
}

pub fn opencode_active() -> Option<HashSet<String>> {
    running_opencode_sessions()
}

pub fn tmux_listing() -> String {
    Command::new("tmux")
        .args(["list-sessions", "-F", SESSION_FORMAT])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default()
}

fn repo_root() -> io::Result<PathBuf> {
    Ok(PathBuf::from(stdout_line(run(
        Command::new("git").args(["rev-parse", "--show-toplevel"])
    )?)))
}

fn clear_marker(marker: &Path) -> io::Result<()> {
    if let Some(dir) = marker.parent() {
        fs::create_dir_all(dir)?;
    }
    let removed = if marker.is_dir() {
        fs::remove_dir_all(marker)
    } else {
        fs::remove_file(marker)
    };
    match removed {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

fn locate_worktree(
    repo: &Path,
    id: &str,
    flag: Option<&str>,
    config: Result<&AppConfig, &String>,
) -> io::Result<(PathBuf, WorktreeDir)> {
    let dir = resolve_worktree_dir(
        repo,
        dirs::home_dir().as_deref(),
        flag,
        std::env::var(WORKTREE_DIR_ENV).ok().as_deref(),
        config
            .map(|config| config.chat.worktree_dir.as_deref())
            .map_err(String::as_str),
    );
    let git = || {
        let mut git = Command::new("git");
        git.arg("-C").arg(repo);
        git
    };
    run(git().args(["worktree", "prune"]))?;
    let listing = run(git().args(["worktree", "list", "--porcelain"]))?;
    let worktree = worktree_for_branch(&String::from_utf8_lossy(&listing.stdout), &branch_name(id))
        .unwrap_or_else(|| worktree_path(&dir.path, id));
    Ok((worktree, dir))
}

fn git_value(repo: &Path, args: &[&str]) -> io::Result<Option<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()?;
    if !output.status.success() {
        return Ok(None);
    }
    let value = stdout_line(output);
    Ok((!value.is_empty()).then_some(value))
}

fn branch_exists(repo: &Path, branch: &str) -> io::Result<bool> {
    Ok(Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--verify", "--quiet"])
        .arg(format!("refs/heads/{branch}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()?
        .success())
}

fn exclude(repo: &Path, lines: &[String]) -> io::Result<()> {
    let common = stdout_line(run(Command::new("git").arg("-C").arg(repo).args([
        "rev-parse",
        "--path-format=absolute",
        "--git-common-dir",
    ]))?);
    let path = Path::new(&common).join("info").join("exclude");
    let current = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let missing: String = lines
        .iter()
        .filter(|line| !current.lines().any(|l| l.trim() == line.as_str()))
        .map(|line| format!("{line}\n"))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let separator = if current.is_empty() || current.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    fs::write(&path, format!("{current}{separator}{missing}"))
}

pub fn running_workers(store_dir: &Path) -> Vec<RunningWorker> {
    let mut workers = store_workers(&tmux_listing(), &store_dir.to_string_lossy());
    workers.extend(
        owned_cursor_ids(store_dir)
            .iter()
            .map(|id| RunningWorker::cursor(id)),
    );
    workers.extend(
        entry_ids(store_dir, OPENCODE_OWNER_DIR)
            .into_iter()
            .map(|id| RunningWorker {
                id,
                session: String::new(),
                opencode: true,
                cursor: true,
            }),
    );
    workers.sort_by(|a, b| a.id.cmp(&b.id));
    workers
}

pub fn capture_pane(session: &str) -> Option<String> {
    let output =
        run(Command::new("tmux").args(["capture-pane", "-p", "-t", &format!("={session}:")]))
            .ok()?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn run(command: &mut Command) -> io::Result<Output> {
    check(command.stdin(Stdio::null()).output()?)
}

fn check(output: Output) -> io::Result<Output> {
    if output.status.success() {
        Ok(output)
    } else {
        Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
    }
}

fn stdout_line(output: Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_store_becomes_absolute() {
        assert!(canonical_store(PathBuf::from("./.memory")).is_absolute());
    }

    #[test]
    fn absolute_store_is_unchanged() {
        assert_eq!(
            canonical_store(PathBuf::from("/s/.memory")),
            PathBuf::from("/s/.memory")
        );
    }

    #[test]
    fn symlinked_store_canonicalizes_to_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        fs::create_dir(&real).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert_eq!(canonical_store(link), canonical_store(real));
    }

    fn workers_in(dir: &Path) -> Workers {
        Workers::new(dir.to_path_buf(), "/bin/ralph".into(), "auto".into(), None)
    }

    #[test]
    fn a_successful_send_writes_the_sent_marker() {
        let dir = tempfile::tempdir().unwrap();
        let workers = workers_in(dir.path());
        let result = workers.mark_sent("A", Ok("delivered".to_string()));
        assert_eq!(result.unwrap(), "delivered");
        assert!(sent_path(&workers.store_dir, "A").exists());
    }

    #[test]
    fn a_failed_send_writes_no_sent_marker() {
        let dir = tempfile::tempdir().unwrap();
        let workers = workers_in(dir.path());
        let result = workers.mark_sent("A", Err(io::Error::other("no pane")));
        assert_eq!(result.unwrap_err().to_string(), "no pane");
        assert!(!sent_path(&workers.store_dir, "A").exists());
    }

    #[test]
    fn opencode_http_workers_are_found_and_owned_by_this_chat() {
        let dir = tempfile::tempdir().unwrap();
        let workers = workers_in(dir.path());
        assert!(!workers.opencode_http("A"));
        assert_eq!(workers.owner("p", "A"), Owner::Missing);
        write_file(&OpencodeFiles::new(dir.path(), "A").owner, "").unwrap();
        write_file(&OpencodeFiles::new(dir.path(), "B").session, "{}").unwrap();
        assert!(workers.opencode_http("A") && workers.opencode_http("B"));
        assert!(matches!(workers.owner("p", "A"), Owner::Mine(s) if s.opencode));
    }

    #[test]
    fn running_workers_list_opencode_http_workers_as_headless() {
        let dir = tempfile::tempdir().unwrap();
        write_file(&OpencodeFiles::new(dir.path(), "A").owner, "").unwrap();
        let running = running_workers(dir.path());
        let worker = running.iter().find(|worker| worker.id == "A").unwrap();
        assert!(worker.opencode && worker.cursor && worker.session.is_empty());
    }

    #[test]
    fn a_reply_to_an_opencode_worker_queues_a_prompt_entry() {
        let dir = tempfile::tempdir().unwrap();
        let workers = workers_in(dir.path());
        write_file(&OpencodeFiles::new(dir.path(), "A").owner, "").unwrap();
        assert_eq!(
            workers.reply("A", "hello", false).unwrap(),
            "Sent to worker A."
        );
        assert_eq!(
            workers.reply("A", "stop", true).unwrap(),
            "Interrupted worker A."
        );
        let files = OpencodeFiles::new(dir.path(), "A");
        let only = |dir: &Path| {
            fs::read_to_string(fs::read_dir(dir).unwrap().next().unwrap().unwrap().path())
        };
        assert_eq!(only(&files.prompt).unwrap(), "hello");
        assert_eq!(only(&files.interrupt).unwrap(), "stop");
        assert!(sent_path(dir.path(), "A").exists());
    }
}
