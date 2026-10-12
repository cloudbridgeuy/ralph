use ralph_core::chat_status::active_sessions;
use ralph_core::worker::{
    branch_name, exclude_lines, opencode_command, opencode_config, session_name, startup_state,
    worker_command, worker_mcp_config, worktree_add_args, worktree_path, Harness, Startup,
    INBOX_DIR, OPENCODE_CONFIG_FILE, OPENCODE_HARNESS, TRUST_KEYS, WORKERS_DIR,
};
use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(20);
const STORE_OPTION: &str = "@ralph_store";
const HARNESS_OPTION: &str = "@ralph_harness";
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
        let session = session_name(id);
        if session_exists(&session)? {
            return Err(io::Error::other(format!("worker {id} is already running")));
        }
        let repo = PathBuf::from(stdout_line(run(
            Command::new("git").args(["rev-parse", "--show-toplevel"])
        )?));
        let worktree = worktree_path(&repo, id);
        exclude(&repo, &exclude_lines(harness))?;
        ensure_worktree(&repo, &worktree, &branch_name(id))?;
        let inbox = self.store_dir.join(INBOX_DIR);
        let command = match harness {
            Harness::Claude => worker_command(&self.write_config(id)?, task),
            Harness::Opencode { model } => {
                let config = opencode_config(model, &self.exe, id, &inbox);
                fs::write(worktree.join(OPENCODE_CONFIG_FILE), config.to_string())?;
                opencode_command(id, task)
            }
        };
        run(Command::new("tmux")
            .args(["new-session", "-d", "-s", &session])
            .arg("-c")
            .arg(&worktree)
            .args(command))?;
        run(Command::new("tmux")
            .args(["set-option", "-t", &format!("={session}:"), STORE_OPTION])
            .arg(&self.store_dir))?;
        if let Harness::Opencode { .. } = harness {
            run(Command::new("tmux").args([
                "set-option",
                "-t",
                &format!("={session}:"),
                HARNESS_OPTION,
                OPENCODE_HARNESS,
            ]))?;
        }
        await_startup(&session, harness)?;
        Ok(format!(
            "Worker {id} started in {} on branch {}. Watch it with: tmux attach -t {session}",
            worktree.display(),
            branch_name(id)
        ))
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

pub fn reply(id: &str, text: &str) -> io::Result<String> {
    let session = session_name(id);
    if !session_exists(&session)? {
        return Err(io::Error::other(format!("worker {id} is not running")));
    }
    let target = format!("={session}:");
    let mut load = Command::new("tmux")
        .args(["load-buffer", "-b", &session, "-"])
        .stdin(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = load.stdin.take() {
        stdin.write_all(text.as_bytes())?;
    }
    check(load.wait_with_output()?)?;
    run(Command::new("tmux").args(["paste-buffer", "-p", "-d", "-b", &session, "-t", &target]))?;
    run(Command::new("tmux").args(["send-keys", "-t", &target, "Enter"]))?;
    Ok(format!("Sent to worker {id}."))
}

fn await_startup(session: &str, harness: &Harness) -> io::Result<()> {
    let target = format!("={session}:");
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    let mut accepted = false;
    while Instant::now() < deadline {
        let pane = run(Command::new("tmux").args(["capture-pane", "-p", "-t", &target]))?;
        match startup_state(harness, &String::from_utf8_lossy(&pane.stdout)) {
            Startup::Ready => return Ok(()),
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
    Err(io::Error::other(format!(
        "worker started but its agent did not come up within {}s; check it with: tmux attach -t {session}",
        STARTUP_TIMEOUT.as_secs()
    )))
}

fn session_exists(session: &str) -> io::Result<bool> {
    Ok(Command::new("tmux")
        .args(["has-session", "-t", &format!("={session}")])
        .stderr(Stdio::null())
        .status()?
        .success())
}

/// Leaves `worktree` checked out on `branch`, reusing whatever an earlier spawn left behind.
fn ensure_worktree(repo: &Path, worktree: &Path, branch: &str) -> io::Result<()> {
    run(Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["worktree", "prune"]))?;
    if worktree.join(".git").exists() {
        return Ok(());
    }
    let branch_exists = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--verify", "--quiet"])
        .arg(format!("refs/heads/{branch}"))
        .stdout(Stdio::null())
        .status()?
        .success();
    run(Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(worktree_add_args(worktree, branch, branch_exists)))?;
    Ok(())
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
