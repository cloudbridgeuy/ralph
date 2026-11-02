use super::spawn::await_startup;
use super::{run, send_to_pane};
use crate::config::AppConfig;
use crate::subprocess::timed_output;
use crate::worker_verify::{step, version_probe, Checks};
use ralph_core::claude_leftovers::configured_dir;
use ralph_core::worker::{
    claude_permission_check, claude_prompt_check, flags_check, probe_text, resolve_claude_shell,
    startup_check, stop_seen, tmux_claude_version, CheckResult, Verdict, VerifyPaths,
    CLAUDE_COMMAND, CLAUDE_VERIFY_MCP, CLAUDE_VERIFY_PERMISSION_PROMPT, VERSION_PROBE_LIMIT,
};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const TURN_LIMIT: Duration = Duration::from_secs(120);
const POLL: Duration = Duration::from_millis(250);
const INPUT_SETTLE: Duration = Duration::from_secs(1);
const REPLY_PROMPT: &str = "Reply with the single word: ok";

struct Session(String);

impl Session {
    fn target(&self) -> String {
        format!("={}:", self.0)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        Command::new("tmux")
            .args(["kill-session", "-t", &format!("={}", self.0)])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .ok();
    }
}

fn prepare(paths: &VerifyPaths) -> io::Result<()> {
    let markers = &paths.markers;
    let marker_dirs = [
        &markers.ready,
        &markers.reported,
        &markers.blocked,
        &markers.waiting,
        &markers.unreported,
        &markers.activity,
    ]
    .into_iter()
    .filter_map(|file| file.parent());
    let dirs = [&paths.work, &paths.attachments, &paths.task_dir]
        .into_iter()
        .map(PathBuf::as_path)
        .chain(marker_dirs);
    for dir in dirs {
        fs::create_dir_all(dir)?;
    }
    fs::write(&paths.mcp_config, CLAUDE_VERIFY_MCP)?;
    fs::write(
        &paths.settings,
        paths.settings_json(&std::env::current_exe()?).to_string(),
    )
}

fn launch(
    dir: &Path,
    config: &Result<AppConfig, String>,
    model: Option<&str>,
) -> Result<(Session, VerifyPaths), String> {
    let dir = fs::canonicalize(dir)
        .map_err(|error| format!("could not resolve {}: {error}", dir.display()))?;
    let paths = VerifyPaths::new(&dir);
    prepare(&paths).map_err(|error| format!("could not prepare the verify folder: {error}"))?;
    let (shell, _) = resolve_claude_shell(
        config
            .as_ref()
            .map(|config| config.claude.shell_setting())
            .map_err(String::as_str),
    );
    let config_dir = config.as_ref().ok().and_then(|config| {
        configured_dir(
            config.claude.config_dir.as_deref(),
            dirs::home_dir().as_deref(),
        )
    });
    let name = format!(
        "ralph-verify-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis())
    );
    run(Command::new("tmux")
        .args(["new-session", "-d", "-s", &name])
        .arg("-c")
        .arg(&paths.work)
        .args(paths.command(shell, config_dir.as_deref(), model)))
    .map_err(|error| format!("could not start the tmux session: {error}"))?;
    Ok((Session(name), paths))
}

fn wait_until(limit: Duration, done: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    while !done() {
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(POLL);
    }
    true
}

fn stopped(paths: &VerifyPaths) -> bool {
    stop_seen(fs::read_to_string(&paths.markers.activity).ok().as_deref())
}

fn submit(session: &Session, paths: &VerifyPaths, text: &str) -> io::Result<()> {
    for marker in [&paths.markers.activity, &paths.markers.blocked] {
        fs::remove_file(marker).or_else(|error| match error.kind() {
            io::ErrorKind::NotFound => Ok(()),
            _ => Err(error),
        })?;
    }
    fs::write(&paths.markers.reported, "")?;
    send_to_pane(&session.0, text)
}

fn prompt_turn(session: &Session, paths: &VerifyPaths) -> CheckResult {
    match submit(session, paths, REPLY_PROMPT) {
        Ok(()) => claude_prompt_check(wait_until(TURN_LIMIT, || stopped(paths)), TURN_LIMIT),
        Err(error) => CheckResult::fail(
            "prompt/reply",
            format!("could not send the prompt: {error}"),
        ),
    }
}

fn permission_turn(session: &Session, paths: &VerifyPaths) -> CheckResult {
    if let Err(error) = submit(session, paths, CLAUDE_VERIFY_PERMISSION_PROMPT) {
        return CheckResult::fail("permission", format!("could not send the prompt: {error}"));
    }
    let blocked = wait_until(TURN_LIMIT, || {
        paths.markers.blocked.exists() || stopped(paths)
    }) && paths.markers.blocked.exists();
    if blocked {
        if let Err(error) =
            run(Command::new("tmux").args(["send-keys", "-t", &session.target(), "Enter"]))
        {
            return CheckResult::fail(
                "permission",
                format!("could not answer the request: {error}"),
            );
        }
    }
    claude_permission_check(
        blocked,
        blocked && wait_until(TURN_LIMIT, || stopped(paths)),
        TURN_LIMIT,
    )
}

pub(crate) fn verify(
    live: bool,
    dir: &Path,
    checks: &Checks,
    config: &Result<AppConfig, String>,
    model: Option<&str>,
) -> Option<String> {
    let probe = version_probe(timed_output(
        Command::new(CLAUDE_COMMAND).arg("--version"),
        VERSION_PROBE_LIMIT,
    ));
    let version = step(checks, "version", tmux_claude_version(&probe), |version| {
        version.clone()
    })?;
    let tmux_probe = version_probe(timed_output(
        Command::new("tmux").arg("-V"),
        VERSION_PROBE_LIMIT,
    ));
    let tmux = step(checks, "tmux", probe_text("tmux -V", &tmux_probe), |text| {
        text.trim().to_string()
    });
    let help_probe = version_probe(timed_output(
        Command::new(CLAUDE_COMMAND).arg("--help"),
        VERSION_PROBE_LIMIT,
    ));
    let flags = probe_text("claude --help", &help_probe)
        .map(|help| flags_check(&help))
        .unwrap_or_else(|message| CheckResult::fail("flags", message));
    let flags_ok = flags.verdict == Verdict::Pass;
    checks.add(flags);
    if tmux.is_none() || !flags_ok {
        return Some(version);
    }
    if !live {
        return Some(version);
    }
    let launched = launch(dir, config, model);
    let (session, paths) = match launched {
        Ok(started) => started,
        Err(message) => {
            checks.add(CheckResult::fail("session start", message));
            return Some(version);
        }
    };
    let started =
        await_startup(&session.0, &paths.markers.ready).map_err(|error| error.to_string());
    let ready = matches!(started, Ok(true));
    checks.add(startup_check(started));
    if ready {
        thread::sleep(INPUT_SETTLE);
        let prompt = prompt_turn(&session, &paths);
        let replied = prompt.verdict == Verdict::Pass;
        checks.add(prompt);
        if replied {
            checks.add(permission_turn(&session, &paths));
        }
    }
    Some(version)
}
