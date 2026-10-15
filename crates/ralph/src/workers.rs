use crate::acp_worker::{caller_owns_cursor, now_secs, owned_cursor_ids, sweep_owner, write_file};
use crate::claude_cleanup::cleanup_worktree_leftovers;
use crate::config::AppConfig;
use ralph_core::chat_status::{active_sessions, store_workers, RunningWorker};
use ralph_core::claude_leftovers::cleans_claude_leftovers;
use ralph_core::worker::{
    base_branch, base_config_key, blocked_path, branch_name, claude_worker_settings, close_handoff,
    close_plan, exclude_lines, marker_paths, opencode_command, opencode_config,
    opencode_session_gone, opencode_session_id, opencode_worker_plugin, project_slug,
    resolve_worktree_dir, session_name, session_owner, spawn_handoff, startup_state,
    tagged_sessions, worker_command, worker_mcp_config, worktree_add_args, worktree_for_branch,
    worktree_path, wt_switch_args, CloseState, CloseStep, CursorFiles, Handoff, Harness, Owner,
    SpawnRequest, Startup, Sweep, TaggedSession, WorkerMarkers, WorktreeDir, WorktreeVia,
    ATTACHMENTS_DIR, HANDOFF_POLL, HARNESS_OPTION, ID_OPTION, INBOX_DIR, OPENCODE_CONFIG_FILE,
    OPENCODE_HARNESS, OPENCODE_PLUGIN_FILE, SESSION_FORMAT, STORE_OPTION, TRUST_KEYS, WORKERS_DIR,
    WORKTREE_DIR_ENV,
};
use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(20);
const STARTUP_POLL: Duration = Duration::from_millis(250);

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

    pub fn spawn(&self, id: &str, task: &str, harness: &Harness) -> io::Result<String> {
        let repo = repo_root()?;
        let project = project_slug(&repo);
        self.refuse_busy_cursor(id, false)?;
        match self.owner(&project, id) {
            Owner::Missing => {}
            Owner::Mine(_) => {
                return Err(io::Error::other(format!("worker {id} is already running")))
            }
            Owner::Foreign(session) => {
                return Err(io::Error::other(format!(
                    "worker {id} is running for another chat (store {})",
                    session.store
                )))
            }
        }
        let session = session_name(&project, id);
        let config = AppConfig::load().map_err(|e| e.to_string());
        let (worktree, worktree_dir) =
            locate_worktree(&repo, id, self.worktree_dir.as_deref(), config.as_ref())?;
        exclude(&repo, &exclude_lines(harness, &repo, &worktree_dir.path))?;
        let via = ensure_worktree(&repo, &worktree, &branch_name(id))?;
        record_base(&repo, id)?;
        let inbox = self.store_dir.join(INBOX_DIR);
        let attachments = self.store_dir.join(ATTACHMENTS_DIR);
        fs::create_dir_all(&attachments)?;
        let markers = WorkerMarkers::new(&self.store_dir, id);
        for marker in marker_paths(&self.store_dir, id) {
            clear_marker(&marker)?;
        }
        let command = match harness {
            Harness::Claude => {
                let settings = self.write_settings(id, &markers)?;
                worker_command(
                    &self.write_config(id)?,
                    &settings,
                    &self.permission_mode,
                    &attachments,
                    task,
                )
            }
            Harness::Opencode { model } => {
                let config = opencode_config(model, &self.exe, id, &inbox, &attachments);
                fs::write(worktree.join(OPENCODE_CONFIG_FILE), config.to_string())?;
                let plugin = worktree.join(OPENCODE_PLUGIN_FILE);
                if let Some(dir) = plugin.parent() {
                    fs::create_dir_all(dir)?;
                }
                fs::write(plugin, opencode_worker_plugin(id, &markers))?;
                opencode_command(id, task)
            }
            Harness::Cursor => {
                self.spawn_cursor(id, &worktree, task)?;
                let started = started_text(id, &worktree, &via, worktree_dir.warning.as_deref());
                return Ok(format!(
                    "{started} It runs headless inside ralph chat (no tmux session)."
                ));
            }
        };
        run(Command::new("tmux")
            .args(["new-session", "-d", "-s", &session])
            .arg("-c")
            .arg(&worktree)
            .args(command))?;
        let target = format!("={session}:");
        run(Command::new("tmux")
            .args(["set-option", "-t", &target, STORE_OPTION])
            .arg(&self.store_dir))?;
        run(Command::new("tmux").args(["set-option", "-t", &target, ID_OPTION, id]))?;
        if let Harness::Opencode { .. } = harness {
            run(Command::new("tmux").args([
                "set-option",
                "-t",
                &target,
                HARNESS_OPTION,
                OPENCODE_HARNESS,
            ]))?;
        }
        let confirmed = await_startup(&session, &markers.ready)?;
        let started = started_text(id, &worktree, &via, worktree_dir.warning.as_deref());
        Ok(if confirmed {
            format!("{started} Watch it with: tmux attach -t {session}")
        } else {
            format!(
                "{started} Its ready signal is not confirmed yet after {}s; check it with: tmux attach -t {session}",
                STARTUP_TIMEOUT.as_secs()
            )
        })
    }

    pub fn reply(&self, id: &str, text: &str) -> io::Result<String> {
        if CursorFiles::new(&self.store_dir, id).owner.exists() {
            return self.answer_permission(id, text);
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
            write_file(&files.prompt, text)?;
            return Ok(format!("Nudged worker {id}."));
        }
        self.reply(id, text)
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
            cursor: CursorFiles::new(&self.store_dir, id).owner.exists(),
        };
        let with_warning = |text: String| {
            [&base.warning, &worktree_dir.warning]
                .into_iter()
                .flatten()
                .fold(text, |text, warning| format!("{text} {warning}"))
        };
        let steps = close_plan(&state, abandon, force)
            .map_err(|refusal| io::Error::other(with_warning(refusal)))?;
        for step in &steps {
            self.close_step(step, &repo, &worktree, id).map_err(|e| {
                io::Error::other(format!("closing worker {id} stopped at {step:?}: {e}"))
            })?;
        }
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
                let output = Command::new("opencode")
                    .args(["api", "session.remove", "--param"])
                    .arg(format!("sessionID={}", opencode_session_id(id)))
                    .stdin(Stdio::null())
                    .output()?;
                if output.status.success()
                    || opencode_session_gone(&String::from_utf8_lossy(&output.stderr))
                {
                    return Ok(());
                }
                check(output)?;
            }
            CloseStep::TerminateCursor => self.terminate_cursor(id)?,
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
            CloseStep::EvictOpencodeLocation => {
                run(Command::new("opencode")
                    .args(["api", "debug.location.evict", "--param"])
                    .arg(format!("location[directory]={}", worktree.display())))?;
            }
            CloseStep::ClearMarkers => {
                for marker in marker_paths(&self.store_dir, id) {
                    clear_marker(&marker)?;
                }
            }
        }
        Ok(())
    }

    fn spawn_cursor(&self, id: &str, worktree: &Path, task: &str) -> io::Result<()> {
        let files = CursorFiles::new(&self.store_dir, id);
        for path in files.all() {
            clear_marker(path)?;
        }
        let request = SpawnRequest {
            worktree: worktree.to_path_buf(),
            exe: self.exe.clone(),
            inbox: self.store_dir.join(INBOX_DIR),
            task: task.to_string(),
            at: now_secs(),
        };
        write_file(
            &files.spawn,
            &serde_json::to_string(&request).map_err(io::Error::other)?,
        )?;
        let ready = WorkerMarkers::new(&self.store_dir, id).ready;
        let started = Instant::now();
        loop {
            let error = fs::read_to_string(&files.error).ok();
            match spawn_handoff(ready.exists(), error.as_deref(), started.elapsed()) {
                Handoff::Done => return Ok(()),
                Handoff::Failed(message) => {
                    clear_marker(&files.error)?;
                    return Err(io::Error::other(message));
                }
                Handoff::TimedOut => {
                    clear_marker(&files.spawn)?;
                    return Err(io::Error::other(
                        "no ralph chat picked up the cursor worker in time; cursor workers start only from a running `ralph chat` with this store",
                    ));
                }
                Handoff::Pending => thread::sleep(HANDOFF_POLL),
            }
        }
    }

    pub fn awaits_permission(&self, id: &str) -> bool {
        CursorFiles::new(&self.store_dir, id).owner.exists()
            && blocked_path(&self.store_dir, id).exists()
    }

    fn answer_permission(&self, id: &str, text: &str) -> io::Result<String> {
        if !self.awaits_permission(id) {
            return Err(io::Error::other(format!(
                "worker {id} is a cursor worker with no permission request waiting; other replies to cursor workers are not supported yet"
            )));
        }
        let files = CursorFiles::new(&self.store_dir, id);
        clear_marker(&files.error)?;
        write_file(&files.answer, text)?;
        let started = Instant::now();
        loop {
            match close_handoff(files.answer.exists(), started.elapsed()) {
                Handoff::Done => break,
                Handoff::TimedOut => {
                    clear_marker(&files.answer)?;
                    return Err(io::Error::other(
                        "ralph chat did not pick up the permission answer in time",
                    ));
                }
                Handoff::Pending | Handoff::Failed(_) => thread::sleep(HANDOFF_POLL),
            }
        }
        match fs::read_to_string(&files.error) {
            Ok(message) => {
                clear_marker(&files.error)?;
                Err(io::Error::other(message))
            }
            Err(_) => Ok(format!("Answered the permission request of worker {id}.")),
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

    fn write_settings(&self, id: &str, markers: &WorkerMarkers) -> io::Result<PathBuf> {
        let dir = self.store_dir.join(WORKERS_DIR);
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{id}.settings.json"));
        fs::write(
            &path,
            claude_worker_settings(&self.exe, markers).to_string(),
        )?;
        Ok(path)
    }

    fn write_config(&self, id: &str) -> io::Result<PathBuf> {
        let dir = self.store_dir.join(WORKERS_DIR);
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{id}.json"));
        let config = worker_mcp_config(&self.exe, id, &self.store_dir.join(INBOX_DIR));
        fs::write(&path, config.to_string())?;
        Ok(path)
    }
}

pub fn canonical_store(path: PathBuf) -> PathBuf {
    fs::canonicalize(&path)
        .or_else(|_| std::path::absolute(&path))
        .unwrap_or(path)
}

pub fn opencode_active() -> Option<HashSet<String>> {
    Command::new("opencode")
        .args(["api", "session.active"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| active_sessions(&String::from_utf8_lossy(&output.stdout)))
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

fn started_text(id: &str, worktree: &Path, via: &WorktreeVia, warning: Option<&str>) -> String {
    let started = format!(
        "Worker {id} started in {} on branch {}. {}",
        worktree.display(),
        branch_name(id),
        via.note()
    );
    match warning {
        Some(warning) => format!("{started} {warning}"),
        None => started,
    }
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
    match fs::remove_file(marker) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

fn await_startup(session: &str, ready: &Path) -> io::Result<bool> {
    let target = format!("={session}:");
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    let mut accepted = false;
    let mut state = Startup::Starting;
    while Instant::now() < deadline {
        if !session_exists(session)? {
            return Err(io::Error::other(format!(
                "worker exited during startup; its tmux session {session} is gone"
            )));
        }
        let pane = run(Command::new("tmux").args(["capture-pane", "-p", "-t", &target]))?;
        state = startup_state(&String::from_utf8_lossy(&pane.stdout), ready.exists());
        match state {
            Startup::Ready => return Ok(true),
            Startup::TrustPrompt if !accepted => {
                run(Command::new("tmux")
                    .args(["send-keys", "-t", &target])
                    .args(TRUST_KEYS))?;
                accepted = true;
            }
            Startup::TrustPrompt | Startup::Starting => {}
        }
        thread::sleep(STARTUP_POLL);
    }
    if state == Startup::TrustPrompt {
        return Err(io::Error::other(format!(
            "worker is stuck on the folder-trust prompt; answer it with: tmux attach -t {session}"
        )));
    }
    Ok(false)
}

fn session_exists(session: &str) -> io::Result<bool> {
    Ok(Command::new("tmux")
        .args(["has-session", "-t", &format!("={session}")])
        .stderr(Stdio::null())
        .status()?
        .success())
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

fn ensure_worktree(repo: &Path, worktree: &Path, branch: &str) -> io::Result<WorktreeVia> {
    if worktree.join(".git").exists() {
        return Ok(WorktreeVia::Existing);
    }
    let wt_error = match Command::new("wt")
        .args(wt_switch_args(
            repo,
            worktree,
            branch,
            branch_exists(repo, branch)?,
        ))
        .stdin(Stdio::null())
        .output()
    {
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        output => match output.and_then(check) {
            _ if worktree.join(".git").exists() => return Ok(WorktreeVia::Wt),
            Ok(_) => Some(format!("it did not create {}", worktree.display())),
            Err(e) => Some(e.to_string()),
        },
    };
    run(Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(worktree_add_args(
            worktree,
            branch,
            branch_exists(repo, branch)?,
        )))?;
    Ok(WorktreeVia::Git { wt_error })
}

fn record_base(repo: &Path, id: &str) -> io::Result<()> {
    let key = base_config_key(id);
    if git_value(repo, &["config", "--get", &key])?.is_some() {
        return Ok(());
    }
    if let Some(base) = git_value(repo, &["symbolic-ref", "-q", "--short", "HEAD"])? {
        run(Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["config", &key, &base]))?;
    }
    Ok(())
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
}
