use crate::config::AppConfig;
use crate::cursor_owner::{
    caller_owns, owned_claude_ids, owned_codex_ids, owned_cursor_ids, sweep_owner, write_file,
};
use crate::opencode_worker::{entry_ids, running_opencode_sessions};
use crate::outcome_log;
use crate::warn::warn_if_err;
use ralph_core::chat_status::{store_workers, RunningWorker};
use ralph_core::worker::{
    activity_path, base_branch, base_config_key, blocked_path, branch_name, capabilities,
    close_clear_paths, close_handoff, close_remove_paths, delivery_line, describe, interrupt_reply,
    legacy_base_unset_warning, lost_marker_name, opencode_close_handoff, parse_interrupt_result,
    project_slug, prompt_entry_name, ready_path, reply_event, resolve_worktree_dir, sent_path,
    session_name, session_owner, session_survived_kill, tagged_sessions, uptake, via_permission,
    worktree_for_branch, worktree_path, ActivityMark, Base, CloseStep, Feature, Handoff,
    HandoffFiles, HarnessId, InterruptOutcome, Owner, Probe, ReplyFacts, Support, Sweep,
    TaggedSession, WorktreeDir, HANDOFF_POLL, HANDOFF_TIMEOUT, OPENCODE_OWNER_DIR, REPLY_CONFIRM,
    SESSION_FORMAT, WORKTREE_DIR_ENV,
};
use ralph_core::worker_check::pane_busy;
use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{self, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

mod close;
mod guard;
pub mod headless;
mod reply;
mod spawn;
mod verify;
pub use spawn::SpawnArgs;
pub(crate) use verify::verify;

#[derive(Clone)]
pub struct Workers {
    store_dir: PathBuf,
    written_store: PathBuf,
    exe: PathBuf,
    permission_mode: String,
    worktree_dir: Option<String>,
    handoff_timeout: Duration,
    confirm_timeout: Duration,
}

enum Watch {
    Pane(String),
    Entry(PathBuf),
}

struct Sent {
    line: String,
    watch: Option<Watch>,
}

impl Sent {
    fn plain(line: String) -> Self {
        Self { line, watch: None }
    }
}

impl Workers {
    pub fn new(
        store_dir: PathBuf,
        exe: PathBuf,
        permission_mode: String,
        worktree_dir: Option<String>,
    ) -> Self {
        Self {
            store_dir: canonical_store(store_dir.clone()),
            written_store: std::path::absolute(&store_dir).unwrap_or(store_dir),
            exe,
            permission_mode,
            worktree_dir,
            handoff_timeout: HANDOFF_TIMEOUT,
            confirm_timeout: REPLY_CONFIRM,
        }
    }

    fn mark_sent<T>(&self, id: &str, sent: io::Result<T>) -> io::Result<T> {
        if sent.is_ok() {
            warn_if_err(
                write_file(&sent_path(&self.store_dir, id), ""),
                format!("marking worker {id} as sent"),
            );
        }
        sent
    }

    pub fn send_reply(&self, id: &str, text: &str) -> io::Result<String> {
        let delivered = self.deliver(id, text, false);
        let sent = self.mark_sent(id, delivered)?;
        Ok(self.finish_reply(id, sent))
    }

    pub fn reply(
        &self,
        id: &str,
        text: &str,
        interrupt: bool,
        request: Option<&str>,
    ) -> io::Result<String> {
        let awaiting = self.awaits_permission(id);
        let delivered = match request.filter(|_| !interrupt) {
            Some(request_id) => self
                .answer_permission(id, request_id, text)
                .map(Sent::plain),
            None => self.deliver(id, text, interrupt),
        };
        let sent = self.mark_sent(id, delivered)?;
        let harness = self
            .headless_files(id)
            .map_or(HarnessId::ClaudeTmux, |(kind, _)| kind);
        outcome_log::record(&self.store_dir, id, |key| {
            reply_event(&ReplyFacts {
                key,
                harness,
                interrupt,
                via_permission: via_permission(request.is_some(), interrupt, awaiting),
                text_bytes: text.len(),
            })
        });
        Ok(self.finish_reply(id, sent))
    }

    fn finish_reply(&self, id: &str, sent: Sent) -> String {
        match sent.watch {
            Some(watch) => self.confirm(id, &watch),
            None => sent.line,
        }
    }

    fn confirm(&self, id: &str, watch: &Watch) -> String {
        let started = Instant::now();
        loop {
            let found = uptake(&self.probe(id, watch));
            let waited = started.elapsed();
            if found.settled() || waited >= self.confirm_timeout {
                return delivery_line(id, found, waited);
            }
            thread::sleep(HANDOFF_POLL);
        }
    }

    fn probe(&self, id: &str, watch: &Watch) -> Probe {
        let (pane_busy, entry_pending) = match watch {
            Watch::Pane(session) => (capture_pane(session).map(|pane| pane_busy(&pane)), None),
            Watch::Entry(entry) => (None, Some(entry.exists())),
        };
        let lost = match watch {
            Watch::Entry(entry) => {
                lost_marker(entry).is_some_and(|marker| fs::remove_file(marker).is_ok())
            }
            Watch::Pane(_) => false,
        };
        Probe {
            pane_busy,
            entry_pending,
            running: fs::read_to_string(activity_path(&self.store_dir, id))
                .ok()
                .and_then(|text| ActivityMark::parse(&text))
                .is_some_and(|mark| mark.busy()),
            sent: sent_path(&self.store_dir, id).exists(),
            lost,
        }
    }

    fn deliver(&self, id: &str, text: &str, interrupt: bool) -> io::Result<Sent> {
        if let Some(sent) = self.deliver_headless(id, text, interrupt, "Sent to") {
            return sent;
        }
        if interrupt {
            return Err(io::Error::other(
                "interrupt needs a Cursor, Codex or opencode worker",
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
        send_to_pane(&session, text)?;
        Ok(Sent {
            line: format!("Sent to worker {id}."),
            watch: Some(Watch::Pane(session)),
        })
    }

    pub fn nudge(&self, id: &str, text: &str) -> io::Result<String> {
        if let Some(queued) = self.deliver_headless(id, text, false, "Nudged") {
            return self.mark_sent(id, queued).map(|sent| sent.line);
        }
        let sent = self.mark_sent(id, self.deliver(id, text, false))?;
        Ok(sent.line)
    }

    fn codex_running(&self, id: &str) -> bool {
        HandoffFiles::codex(&self.store_dir, id).owner.exists()
    }

    fn opencode_http(&self, id: &str) -> bool {
        let files = HandoffFiles::opencode(&self.store_dir, id);
        files.owner.exists() || files.session.exists()
    }

    fn claude_running(&self, id: &str) -> bool {
        HandoffFiles::claude(&self.store_dir, id).owner.exists()
    }

    fn headless_files(&self, id: &str) -> Option<(HarnessId, HandoffFiles)> {
        let cursor = HandoffFiles::cursor(&self.store_dir, id);
        if cursor.owner.exists() {
            return Some((HarnessId::Cursor, cursor));
        }
        let codex = HandoffFiles::codex(&self.store_dir, id);
        if codex.owner.exists() {
            return Some((HarnessId::Codex, codex));
        }
        if self.claude_running(id) {
            let files = HandoffFiles::claude(&self.store_dir, id);
            return Some((HarnessId::Claude, files));
        }
        self.opencode_http(id).then(|| {
            (
                HarnessId::Opencode,
                HandoffFiles::opencode(&self.store_dir, id),
            )
        })
    }

    fn deliver_headless(
        &self,
        id: &str,
        text: &str,
        interrupt: bool,
        verb: &str,
    ) -> Option<io::Result<Sent>> {
        let (kind, files) = self.headless_files(id)?;
        let caps = capabilities(kind);
        Some(
            if interrupt && caps.support(Feature::Interrupt) == Support::None {
                Err(io::Error::other(describe(&caps, Feature::Interrupt)))
            } else if interrupt {
                self.queue_entry(&files.interrupt, text).map(|entry| {
                    Sent::plain(interrupt_reply(
                        id,
                        self.await_interrupt_result(&files.interrupt_result, &entry),
                    ))
                })
            } else if self.awaits_permission(id) {
                self.answer_permission(id, "", text).map(Sent::plain)
            } else {
                self.queue_prompt(&files.prompt, id, text, verb)
            },
        )
    }

    fn await_interrupt_result(&self, results: &Path, entry: &str) -> Option<InterruptOutcome> {
        let result = results.join(entry);
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

    fn queue_prompt(&self, dir: &Path, id: &str, text: &str, verb: &str) -> io::Result<Sent> {
        let entry = self.queue_entry(dir, text)?;
        Ok(Sent {
            line: format!("{verb} worker {id}."),
            watch: Some(Watch::Entry(dir.join(entry))),
        })
    }

    fn queue_entry(&self, dir: &Path, text: &str) -> io::Result<String> {
        let entry = prompt_entry_name(now_nanos(), process::id());
        let pending = dir.join(format!(".{entry}"));
        write_file(&pending, text)?;
        fs::rename(&pending, dir.join(&entry))?;
        Ok(entry)
    }

    pub fn stop_harness(&self, id: &str) -> io::Result<String> {
        let repo = repo_root()?;
        match self.owner(&project_slug(&repo), id) {
            Owner::Mine(session) => {
                self.close_step(&CloseStep::KillSession(session.name), &repo, &repo, id)?;
                Ok(format!(
                    "Stopped worker {id}: kept its worktree and branch {}.",
                    branch_name(id)
                ))
            }
            Owner::Missing => Ok(format!("Worker {id} is not running.")),
            Owner::Foreign(session) => Err(io::Error::other(format!(
                "worker {id} belongs to another chat (store {})",
                session.store
            ))),
        }
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
                if HandoffFiles::cursor(&self.store_dir, id).owner.exists() {
                    self.terminate_headless(&HandoffFiles::cursor(&self.store_dir, id), "cursor")?;
                }
            }
            CloseStep::TerminateCodex => {
                if self.codex_running(id) {
                    self.terminate_headless(&HandoffFiles::codex(&self.store_dir, id), "codex")?;
                }
            }
            CloseStep::TerminateClaude => {
                if self.claude_running(id) {
                    let files = HandoffFiles::claude(&self.store_dir, id);
                    self.terminate_headless(&files, "claude")?;
                }
            }
            CloseStep::KillSession(session) => {
                let ready = ready_path(&self.store_dir, id);
                clear_marker(&ready)?;
                let killed =
                    run(Command::new("tmux").args(["kill-session", "-t", &format!("={session}")]));
                if session_survived_kill(killed.is_ok(), spawn::session_exists(session).ok()) {
                    write_file(&ready, "")?;
                    killed?;
                    return Err(io::Error::other(format!(
                        "tmux session {session} is still alive after kill-session"
                    )));
                }
                killed?;
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
                for path in close_clear_paths(&self.store_dir, id) {
                    clear_marker(&path)?;
                }
                for path in close_remove_paths(&self.store_dir, id) {
                    warn_if_err(
                        clear_marker(&path),
                        format!("Failed to remove {}", path.display()),
                    );
                }
            }
        }
        Ok(())
    }

    pub fn awaits_permission(&self, id: &str) -> bool {
        self.headless_files(id).is_some() && blocked_path(&self.store_dir, id).exists()
    }

    fn terminate_opencode(&self, id: &str) -> io::Result<()> {
        let files = HandoffFiles::opencode(&self.store_dir, id);
        clear_marker(&files.error)?;
        write_file(&files.close, "")?;
        let started = Instant::now();
        loop {
            if let Ok(message) = fs::read_to_string(&files.error) {
                clear_marker(&files.error)?;
                return Err(io::Error::other(message));
            }
            match opencode_close_handoff(
                files.owner.exists(),
                files.session.exists(),
                started.elapsed(),
            ) {
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

    fn refuse_busy_headless(&self, id: &str, closing: bool) -> io::Result<()> {
        for harness in [HarnessId::Cursor, HarnessId::Codex, HarnessId::Claude] {
            let files = match harness {
                HarnessId::Codex => HandoffFiles::codex(&self.store_dir, id),
                HarnessId::Claude => HandoffFiles::claude(&self.store_dir, id),
                _ => HandoffFiles::cursor(&self.store_dir, id),
            };
            match sweep_owner(&files) {
                Some(Sweep::Busy) if closing && caller_owns(&files) => {}
                Some(Sweep::Busy) => {
                    return Err(io::Error::other(format!(
                        "worker {id} is a {} worker owned by another running ralph chat",
                        harness.as_str()
                    )))
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn terminate_headless(&self, files: &HandoffFiles, name: &str) -> io::Result<()> {
        write_file(&files.close, "")?;
        let started = Instant::now();
        loop {
            match close_handoff(files.owner.exists(), started.elapsed()) {
                Handoff::Done => return Ok(()),
                Handoff::TimedOut => {
                    clear_marker(&files.close)?;
                    return Err(io::Error::other(format!(
                        "ralph chat did not confirm stopping the {name} worker in time"
                    )));
                }
                Handoff::Pending | Handoff::Failed(_) => thread::sleep(HANDOFF_POLL),
            }
        }
    }

    fn owner(&self, project: &str, id: &str) -> Owner {
        if self.headless_files(id).is_some() {
            return Owner::Mine(TaggedSession {
                name: session_name(project, id),
                store: self.store_dir.to_string_lossy().into_owned(),
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

pub fn usage_store(store: &Path) -> String {
    canonical_store(store.to_path_buf())
        .to_string_lossy()
        .into_owned()
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

fn lost_marker(entry: &Path) -> Option<PathBuf> {
    let name = entry.file_name()?.to_string_lossy();
    Some(entry.with_file_name(lost_marker_name(&name)))
}

fn repo_root() -> io::Result<PathBuf> {
    Ok(PathBuf::from(stdout_line(run(
        Command::new("git").args(["rev-parse", "--show-toplevel"])
    )?)))
}

fn now_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX)
        })
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

fn resolve_base_branch(repo: &Path, config: &Result<AppConfig, String>) -> io::Result<Base> {
    Ok(base_branch(
        config
            .as_ref()
            .map(|config| config.chat.worker_base.as_deref())
            .map_err(String::as_str),
        git_value(repo, &["symbolic-ref", "-q", "refs/remotes/origin/HEAD"])?.as_deref(),
    ))
}

fn unset_legacy_base(repo: &Path, id: &str) -> Option<String> {
    let exit_code = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["config", "--unset", &base_config_key(id)])
        .stdin(Stdio::null())
        .output()
        .ok()
        .and_then(|output| output.status.code());
    legacy_base_unset_warning(id, exit_code)
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
            .map(|id| RunningWorker::headless(id, HarnessId::Cursor)),
    );
    workers.extend(
        owned_codex_ids(store_dir)
            .iter()
            .map(|id| RunningWorker::headless(id, HarnessId::Codex)),
    );
    workers.extend(
        owned_claude_ids(store_dir)
            .iter()
            .map(|id| RunningWorker::headless(id, HarnessId::Claude)),
    );
    workers.extend(
        entry_ids(store_dir, OPENCODE_OWNER_DIR)
            .into_iter()
            .map(|id| RunningWorker {
                id,
                session: String::new(),
                kind: HarnessId::Opencode,
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

fn send_to_pane(session: &str, text: &str) -> io::Result<()> {
    let target = format!("={session}:");
    let mut load = Command::new("tmux")
        .args(["load-buffer", "-b", session, "-"])
        .stdin(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = load.stdin.take() {
        stdin.write_all(text.as_bytes())?;
    }
    check(load.wait_with_output()?)?;
    run(Command::new("tmux").args(["paste-buffer", "-p", "-d", "-b", session, "-t", &target]))?;
    run(Command::new("tmux").args(["send-keys", "-t", &target, "Enter"]))?;
    Ok(())
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
    use std::time::Duration;

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

    #[test]
    fn usage_store_resolves_a_symlinked_store_to_the_worker_form() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let real = root.join("real");
        fs::create_dir(&real).unwrap();
        let link = root.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert_eq!(usage_store(&link), usage_store(&real));
        assert_eq!(usage_store(&link), canonical_store(real).to_string_lossy());
    }

    #[test]
    fn usage_store_collapses_parent_segments() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        fs::create_dir_all(root.join("a/b")).unwrap();
        assert_eq!(
            usage_store(&root.join("a/b/../b")),
            usage_store(&root.join("a/b"))
        );
    }

    #[test]
    fn a_relative_store_is_written_as_an_absolute_path() {
        let workers = Workers::new(
            PathBuf::from("rel/store"),
            "/bin/ralph".into(),
            "auto".into(),
            None,
        );
        assert!(workers.written_store.is_absolute());
        assert!(workers.written_store.ends_with("rel/store"));
    }

    #[test]
    fn usage_store_of_a_missing_dir_is_absolute() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing");
        assert!(Path::new(&usage_store(&missing)).is_absolute());
    }

    pub(super) fn workers_in(dir: &Path) -> Workers {
        Workers {
            confirm_timeout: Duration::ZERO,
            ..Workers::new(dir.to_path_buf(), "/bin/ralph".into(), "auto".into(), None)
        }
    }

    #[test]
    fn probe_sees_and_clears_a_lost_marker() {
        let dir = tempfile::tempdir().unwrap();
        let workers = workers_in(dir.path());
        let entry = dir.path().join("0001");
        let marker = dir
            .path()
            .join(ralph_core::worker::lost_marker_name("0001"));
        write_file(&marker, "").unwrap();
        let probe = workers.probe("A", &Watch::Entry(entry));
        assert!(probe.lost);
        assert!(!marker.exists());
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
        let result = workers.mark_sent("A", Err::<String, _>(io::Error::other("no pane")));
        assert_eq!(result.unwrap_err().to_string(), "no pane");
        assert!(!sent_path(&workers.store_dir, "A").exists());
    }

    #[test]
    fn opencode_http_workers_are_found_and_owned_by_this_chat() {
        let dir = tempfile::tempdir().unwrap();
        let workers = workers_in(dir.path());
        assert!(!workers.opencode_http("A"));
        assert_eq!(workers.owner("p", "A"), Owner::Missing);
        write_file(&HandoffFiles::opencode(dir.path(), "A").owner, "").unwrap();
        write_file(&HandoffFiles::opencode(dir.path(), "B").session, "{}").unwrap();
        assert!(workers.opencode_http("A") && workers.opencode_http("B"));
        assert!(matches!(workers.owner("p", "A"), Owner::Mine(_)));
    }

    #[test]
    fn running_workers_list_opencode_http_workers_as_headless() {
        let dir = tempfile::tempdir().unwrap();
        write_file(&HandoffFiles::opencode(dir.path(), "A").owner, "").unwrap();
        let running = running_workers(dir.path());
        let worker = running.iter().find(|worker| worker.id == "A").unwrap();
        assert!(worker.kind == HarnessId::Opencode && worker.session.is_empty());
    }

    #[test]
    fn a_reply_reports_a_running_turn_as_busy() {
        let dir = tempfile::tempdir().unwrap();
        let workers = workers_in(dir.path());
        write_file(&HandoffFiles::opencode(dir.path(), "A").owner, "").unwrap();
        write_file(&activity_path(dir.path(), "A"), "1 running").unwrap();
        assert_eq!(
            workers.reply("A", "hello", false, None).unwrap(),
            "Sent to worker A; busy."
        );
    }

    #[test]
    fn a_reply_to_an_opencode_worker_queues_a_prompt_entry() {
        let dir = tempfile::tempdir().unwrap();
        let workers = workers_in(dir.path());
        write_file(&HandoffFiles::opencode(dir.path(), "A").owner, "").unwrap();
        assert_eq!(
            workers.reply("A", "hello", false, None).unwrap(),
            "Sent to worker A; no reaction after 0s."
        );
        let files = HandoffFiles::opencode(dir.path(), "A");
        let only = |dir: &Path| {
            fs::read_to_string(fs::read_dir(dir).unwrap().next().unwrap().unwrap().path())
        };
        assert_eq!(only(&files.prompt).unwrap(), "hello");
        let (interrupts, results) = (files.interrupt.clone(), files.interrupt_result.clone());
        let host = thread::spawn(move || loop {
            if let Some(entry) = fs::read_dir(&interrupts).ok().and_then(|entries| {
                entries
                    .filter_map(Result::ok)
                    .find(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
            }) {
                write_file(&results.join(entry.file_name()), "cancelled").unwrap();
                return;
            }
            thread::sleep(Duration::from_millis(10));
        });
        assert_eq!(
            workers.reply("A", "stop", true, Some("per_1")).unwrap(),
            interrupt_reply("A", Some(InterruptOutcome::Cancelled))
        );
        host.join().unwrap();
        assert_eq!(only(&files.interrupt).unwrap(), "stop");
        assert!(sent_path(dir.path(), "A").exists());
    }

    #[test]
    fn only_a_reply_writes_a_log_row_and_a_nudge_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let workers = workers_in(dir.path());
        write_file(&HandoffFiles::opencode(dir.path(), "A").owner, "").unwrap();
        let log = ralph_core::worker::workers_log_path(dir.path());
        workers.send_reply("A", "hello").unwrap();
        assert!(!log.exists());
        workers.reply("A", "hello", false, None).unwrap();
        let text = fs::read_to_string(&log).unwrap();
        assert_eq!(text.lines().count(), 1);
        assert!(text.contains("\"event\":\"reply\""));
    }
}
