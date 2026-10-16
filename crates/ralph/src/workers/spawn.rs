use super::{
    branch_exists, check, clear_marker, exclude, git_value, locate_worktree, repo_root, run,
    Workers,
};
use crate::acp_worker::{now_secs, write_file, AGENT_COMMAND};
use crate::config::AppConfig;
use ralph_core::worker::{
    base_config_key, branch_name, claude_worker_settings, exclude_lines, marker_paths,
    opencode_config, project_slug, rollback_plan, rollback_report, session_name, spawn_handoff,
    startup_state, worker_command, worker_mcp_config, worktree_add_args, wt_switch_args,
    CursorFiles, Handoff, Harness, OpencodeFiles, OpencodeSpawn, Owner, Rollback, SpawnFootprint,
    SpawnRequest, Startup, WorkerMarkers, WorktreeVia, ATTACHMENTS_DIR, HANDOFF_POLL, ID_OPTION,
    INBOX_DIR, OPENCODE_CONFIG_FILE, STORE_OPTION, TRUST_KEYS, WORKERS_DIR,
};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(20);
const STARTUP_POLL: Duration = Duration::from_millis(250);

impl Workers {
    pub fn spawn(&self, id: &str, task: &str, harness: &Harness) -> io::Result<String> {
        let started = self.start(id, task, harness);
        self.mark_sent(id, started)
    }

    fn start(&self, id: &str, task: &str, harness: &Harness) -> io::Result<String> {
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
        self.preflight(id, harness)?;
        let session = session_name(&project, id);
        let config = AppConfig::load().map_err(|e| e.to_string());
        let (worktree, worktree_dir) =
            locate_worktree(&repo, id, self.worktree_dir.as_deref(), config.as_ref())?;
        exclude(&repo, &exclude_lines(harness, &repo, &worktree_dir.path))?;
        let branch = branch_name(id);
        let created_worktree = !worktree.join(".git").exists();
        let created_branch = !branch_exists(&repo, &branch)?;
        let via = ensure_worktree(&repo, &worktree, &branch)?;
        let created_at = git_value(
            &repo,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch}"),
            ],
        )
        .ok()
        .flatten();
        let launch = Launch {
            id,
            task,
            harness,
            worktree: &worktree,
            session: &session,
            started: started_text(id, &worktree, &via, worktree_dir.warning.as_deref()),
        };
        self.launch(&repo, &launch).map_err(|error| {
            let footprint = SpawnFootprint {
                created_worktree,
                created_branch,
                harness_running: self.harness_running(&launch),
                commits_past_base: created_at
                    .as_deref()
                    .and_then(|start| commits_past(&repo, start, &branch)),
            };
            let failures = roll_back(&repo, &launch, &rollback_plan(&footprint));
            io::Error::other(rollback_report(&error.to_string(), &failures))
        })
    }

    fn preflight(&self, id: &str, harness: &Harness) -> io::Result<()> {
        match harness {
            Harness::Claude => {
                require_on_path("tmux")?;
                require_on_path("zsh")?;
                let found = Command::new("zsh")
                    .args(["-lic", "command -v claude"])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()?
                    .success();
                if found {
                    Ok(())
                } else {
                    Err(io::Error::other(
                        "claude not found on PATH (checked in a zsh login shell)",
                    ))
                }
            }
            Harness::Opencode { .. } => self.preflight_opencode(id),
            Harness::Cursor => require_on_path(AGENT_COMMAND),
        }
    }

    fn launch(&self, repo: &Path, launch: &Launch) -> io::Result<String> {
        let Launch {
            id,
            task,
            harness,
            worktree,
            session,
            started,
        } = launch;
        record_base(repo, id)?;
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
                self.spawn_opencode(id, worktree, model, task)?;
                return Ok(format!(
                    "{started} It runs headless on the shared opencode server inside ralph chat (no tmux session)."
                ));
            }
            Harness::Cursor => {
                self.spawn_cursor(id, worktree, task)?;
                return Ok(format!(
                    "{started} It runs headless inside ralph chat (no tmux session)."
                ));
            }
        };
        run(Command::new("tmux")
            .args(["new-session", "-d", "-s", session])
            .arg("-c")
            .arg(worktree)
            .args(command))?;
        let target = format!("={session}:");
        run(Command::new("tmux")
            .args(["set-option", "-t", &target, STORE_OPTION])
            .arg(&self.store_dir))?;
        run(Command::new("tmux").args(["set-option", "-t", &target, ID_OPTION, id]))?;
        let confirmed = await_startup(session, &markers.ready)?;
        Ok(if confirmed {
            format!("{started} Watch it with: tmux attach -t {session}")
        } else {
            format!(
                "{started} Its ready signal is not confirmed yet after {}s; check it with: tmux attach -t {session}",
                STARTUP_TIMEOUT.as_secs()
            )
        })
    }

    fn harness_running(&self, launch: &Launch) -> bool {
        match launch.harness {
            Harness::Claude => session_exists(launch.session).unwrap_or(true),
            Harness::Opencode { .. } => OpencodeFiles::new(&self.store_dir, launch.id)
                .owner
                .exists(),
            Harness::Cursor => CursorFiles::new(&self.store_dir, launch.id).owner.exists(),
        }
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
        self.await_spawn(&files.spawn, &ready, &files.error, "cursor")
    }

    fn spawn_opencode(&self, id: &str, worktree: &Path, model: &str, task: &str) -> io::Result<()> {
        let files = OpencodeFiles::new(&self.store_dir, id);
        for path in files.requests() {
            clear_marker(path)?;
        }
        let request = OpencodeSpawn {
            worktree: worktree.to_path_buf(),
            inbox: self.store_dir.join(INBOX_DIR),
            model: model.to_string(),
            task: task.to_string(),
            at: now_secs(),
        };
        write_file(&files.spawn, &request.render())?;
        let ready = WorkerMarkers::new(&self.store_dir, id).ready;
        self.await_spawn(&files.spawn, &ready, &files.error, "opencode")
    }

    fn preflight_opencode(&self, id: &str) -> io::Result<()> {
        let files = OpencodeFiles::new(&self.store_dir, id);
        for path in [&files.preflight, &files.healthy, &files.error] {
            clear_marker(path)?;
        }
        write_file(&files.preflight, &now_secs().to_string())?;
        self.await_spawn(&files.preflight, &files.healthy, &files.error, "opencode")
            .and_then(|()| clear_marker(&files.healthy))
    }

    fn await_spawn(
        &self,
        spawn: &Path,
        ready: &Path,
        error: &Path,
        harness: &str,
    ) -> io::Result<()> {
        let started = Instant::now();
        loop {
            let failure = fs::read_to_string(error).ok();
            match spawn_handoff(ready.exists(), failure.as_deref(), started.elapsed()) {
                Handoff::Done => return Ok(()),
                Handoff::Failed(message) => {
                    clear_marker(error)?;
                    return Err(io::Error::other(message));
                }
                Handoff::TimedOut => {
                    clear_marker(spawn)?;
                    return Err(io::Error::other(format!(
                        "no ralph chat picked up the {harness} worker in time; {harness} workers start only from a running `ralph chat` with this store"
                    )));
                }
                Handoff::Pending => thread::sleep(HANDOFF_POLL),
            }
        }
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

struct Launch<'a> {
    id: &'a str,
    task: &'a str,
    harness: &'a Harness,
    worktree: &'a Path,
    session: &'a str,
    started: String,
}

fn require_on_path(program: &str) -> io::Result<()> {
    let found = std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()));
    if found {
        Ok(())
    } else {
        Err(io::Error::other(format!("{program} not found on PATH")))
    }
}

fn commits_past(repo: &Path, start: &str, branch: &str) -> Option<u64> {
    git_value(
        repo,
        &["rev-list", "--count", &format!("{start}..{branch}")],
    )
    .ok()
    .flatten()
    .and_then(|count| count.parse().ok())
}

fn roll_back(repo: &Path, launch: &Launch, plan: &Rollback) -> Vec<String> {
    let mut failures = Vec::new();
    if plan.remove_worktree {
        let removed = run(Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["worktree", "remove"])
            .arg(launch.worktree));
        if let Err(e) = removed {
            failures.push(format!(
                "could not remove {}: {e}",
                launch.worktree.display()
            ));
            return failures;
        }
    }
    if plan.delete_branch {
        let branch = branch_name(launch.id);
        if let Err(e) = run(Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["branch", "-D", &branch]))
        {
            failures.push(format!("could not delete branch {branch}: {e}"));
        }
    }
    failures
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
