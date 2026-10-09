use ralph_core::worker::{
    branch_name, claude_args, session_name, startup_state, tmux_env_args, worker_mcp_config,
    worktree_path, Startup, INBOX_DIR, TRUST_KEYS, WORKERS_DIR, WORKTREE_DIR,
};
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
        Self { store_dir, exe }
    }

    pub fn spawn(&self, id: &str, task: &str) -> io::Result<String> {
        let session = session_name(id);
        if session_exists(&session)? {
            return Err(io::Error::other(format!("worker {id} is already running")));
        }
        let repo = PathBuf::from(stdout_line(run(
            Command::new("git").args(["rev-parse", "--show-toplevel"])
        )?));
        let worktree = worktree_path(&repo, id);
        exclude_worktrees(&repo)?;
        run(Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["worktree", "add", "-b", &branch_name(id)])
            .arg(&worktree))?;
        let config = self.write_config(id)?;
        run(Command::new("tmux")
            .args(["new-session", "-d", "-s", &session])
            .args(tmux_env_args(std::env::vars_os()))
            .arg("-c")
            .arg(&worktree)
            .args(claude_args(&config, task)))?;
        await_startup(&session)?;
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

/// Waits for Claude's UI, accepting the folder-trust prompt if it shows up.
fn await_startup(session: &str) -> io::Result<()> {
    let target = format!("={session}:");
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    let mut accepted = false;
    while Instant::now() < deadline {
        let pane = run(Command::new("tmux").args(["capture-pane", "-p", "-t", &target]))?;
        match startup_state(&String::from_utf8_lossy(&pane.stdout)) {
            Startup::Ready => return Ok(()),
            // A second Down could wrap the selection back to "No, exit".
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
        "worker started but Claude did not come up within {}s; check it with: tmux attach -t {session}",
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

fn exclude_worktrees(repo: &Path) -> io::Result<()> {
    let common = stdout_line(run(Command::new("git").arg("-C").arg(repo).args([
        "rev-parse",
        "--path-format=absolute",
        "--git-common-dir",
    ]))?);
    let path = Path::new(&common).join("info").join("exclude");
    let line = format!("/{WORKTREE_DIR}/");
    let current = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    if current.lines().any(|l| l.trim() == line) {
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
    fs::write(&path, format!("{current}{separator}{line}\n"))
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
