use crate::config::AppConfig;
use ralph_core::chat_status::active_sessions;
use ralph_core::worker::{
    base_branch, base_config_key, branch_name, claude_worker_settings, close_plan, exclude_lines,
    opencode_command, opencode_config, opencode_session_gone, opencode_session_id,
    opencode_worker_plugin, project_slug, ready_path, reported_path, session_name, session_owner,
    startup_state, tagged_sessions, worker_command, worker_mcp_config, worktree_add_args,
    worktree_path, CloseState, CloseStep, Harness, Owner, Startup, HARNESS_OPTION, ID_OPTION,
    INBOX_DIR, OPENCODE_CONFIG_FILE, OPENCODE_HARNESS, OPENCODE_PLUGIN_FILE, SESSION_FORMAT,
    STORE_OPTION, TRUST_KEYS, WORKERS_DIR,
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
}

impl Workers {
    pub fn new(store_dir: PathBuf, exe: PathBuf) -> Self {
        Self {
            store_dir: absolute_or_same(store_dir),
            exe,
        }
    }

    pub fn spawn(&self, id: &str, task: &str, harness: &Harness) -> io::Result<String> {
        let repo = repo_root()?;
        let project = project_slug(&repo);
        match self.owner(&project, id) {
            Owner::Missing => {}
            Owner::Mine(_) => {
                return Err(io::Error::other(format!("worker {id} is already running")))
            }
            Owner::Foreign(store) => {
                return Err(io::Error::other(format!(
                    "worker {id} is running for another chat (store {store})"
                )))
            }
        }
        let session = session_name(&project, id);
        let worktree = worktree_path(&repo, id);
        exclude(&repo, &exclude_lines(harness))?;
        ensure_worktree(&repo, &worktree, &branch_name(id))?;
        record_base(&repo, id)?;
        let inbox = self.store_dir.join(INBOX_DIR);
        let ready = ready_path(&self.store_dir, id);
        let reported = reported_path(&self.store_dir, id);
        clear_marker(&ready)?;
        clear_marker(&reported)?;
        let command = match harness {
            Harness::Claude => {
                let settings = self.write_settings(id, &ready, &reported)?;
                worker_command(&self.write_config(id)?, &settings, task)
            }
            Harness::Opencode { model } => {
                let config = opencode_config(model, &self.exe, id, &inbox);
                fs::write(worktree.join(OPENCODE_CONFIG_FILE), config.to_string())?;
                let plugin = worktree.join(OPENCODE_PLUGIN_FILE);
                if let Some(dir) = plugin.parent() {
                    fs::create_dir_all(dir)?;
                }
                fs::write(plugin, opencode_worker_plugin(id, &ready, &reported))?;
                opencode_command(id, task)
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
        let confirmed = await_startup(&session, &ready)?;
        let started = format!(
            "Worker {id} started in {} on branch {}.",
            worktree.display(),
            branch_name(id)
        );
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
        let session = match self.owner(&project_slug(&repo_root()?), id) {
            Owner::Mine(session) => session.name,
            Owner::Missing => return Err(io::Error::other(format!("worker {id} is not running"))),
            Owner::Foreign(store) => {
                return Err(io::Error::other(format!(
                    "worker {id} belongs to another chat (store {store})"
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

    pub fn close(&self, id: &str, abandon: bool) -> io::Result<String> {
        let repo = repo_root()?;
        let owner = self.owner(&project_slug(&repo), id);
        let worktree = worktree_path(&repo, id);
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
        let configured = AppConfig::load()
            .map(|config| config.chat.worker_base)
            .map_err(|e| e.to_string());
        let base = base_branch(
            configured
                .as_ref()
                .map(Option::as_deref)
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
        let opencode = matches!(&owner, Owner::Mine(session) if session.opencode)
            || worktree.join(OPENCODE_CONFIG_FILE).exists();
        let state = CloseState {
            id: id.to_string(),
            owner,
            dirty_worktree,
            base: base.branch.clone(),
            unmerged_commits,
            opencode,
        };
        let with_warning = |text: String| match &base.warning {
            Some(warning) => format!("{text} {warning}"),
            None => text,
        };
        let steps = close_plan(&state, abandon)
            .map_err(|refusal| io::Error::other(with_warning(refusal)))?;
        for step in &steps {
            self.close_step(step, &repo, id).map_err(|e| {
                io::Error::other(format!("closing worker {id} stopped at {step:?}: {e}"))
            })?;
        }
        let note = if opencode {
            " Its opencode plugin keeps running in the opencode service until that restarts."
        } else {
            ""
        };
        Ok(with_warning(format!(
            "Closed worker {id}: removed its sessions, worktree and branch {branch}.{note}"
        )))
    }

    fn close_step(&self, step: &CloseStep, repo: &Path, id: &str) -> io::Result<()> {
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
            CloseStep::KillSession(session) => {
                run(Command::new("tmux").args(["kill-session", "-t", &format!("={session}")]))?;
            }
            CloseStep::RemoveWorktree { force } => {
                let mut remove = git();
                remove.args(["worktree", "remove"]);
                if *force {
                    remove.arg("--force");
                }
                run(remove.arg(worktree_path(repo, id)))?;
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
                    .arg(format!(
                        "location[directory]={}",
                        worktree_path(repo, id).display()
                    )))?;
            }
            CloseStep::ClearMarkers => {
                clear_marker(&ready_path(&self.store_dir, id))?;
                clear_marker(&reported_path(&self.store_dir, id))?;
            }
        }
        Ok(())
    }

    fn owner(&self, project: &str, id: &str) -> Owner {
        session_owner(
            &tagged_sessions(&tmux_listing()),
            &self.store_dir.to_string_lossy(),
            project,
            id,
        )
    }

    fn write_settings(&self, id: &str, ready: &Path, reported: &Path) -> io::Result<PathBuf> {
        let dir = self.store_dir.join(WORKERS_DIR);
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{id}.settings.json"));
        fs::write(
            &path,
            claude_worker_settings(&self.exe, ready, reported).to_string(),
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

/// Workers run inside `.worktrees/<id>`, so relative store paths would resolve wrongly there.
pub fn absolute_or_same(path: PathBuf) -> PathBuf {
    std::path::absolute(&path).unwrap_or(path)
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

fn ensure_worktree(repo: &Path, worktree: &Path, branch: &str) -> io::Result<()> {
    run(Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["worktree", "prune"]))?;
    if worktree.join(".git").exists() {
        return Ok(());
    }
    run(Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(worktree_add_args(
            worktree,
            branch,
            branch_exists(repo, branch)?,
        )))?;
    Ok(())
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
        assert!(absolute_or_same(PathBuf::from("./.memory")).is_absolute());
    }

    #[test]
    fn absolute_store_is_unchanged() {
        assert_eq!(
            absolute_or_same(PathBuf::from("/s/.memory")),
            PathBuf::from("/s/.memory")
        );
    }
}
