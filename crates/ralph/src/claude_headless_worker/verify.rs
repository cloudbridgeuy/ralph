use super::{random_bytes, Arc, Command, Duration, Mutex};
use crate::acp_log::CappedLog;
use crate::codex_worker::pipes::{logged_stdin, pipe_lines};
use crate::codex_worker::Stdin;
use crate::config::AppConfig;
use crate::subprocess::timed_output;
use crate::transport::stdio::{spawn_piped, stop_child};
use crate::watches::locked;
use crate::worker_verify::{step, version_probe, Budget, Checks};
use ralph_core::claude_leftovers::configured_dir;
use ralph_core::stream::user_line;
use ralph_core::worker::{
    argv_check, cancel_check, claude_headless_args, claude_probed_version, claude_verify_spawns,
    fold_claude_line, init_check, permission_check, prompt_check, uuid_v4, ClaudeFold, ClaudePlan,
    ClaudeSession, ClaudeSpawn, PermissionMode, PermissionSetup, CLAUDE_COMMAND, CLAUDE_ENDED,
    CLAUDE_GATE, CLAUDE_INTERRUPTED, CLAUDE_PERMISSION_PROMPT, VERSION_PROBE_LIMIT,
};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, ChildStderr};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Instant;

const SETTLE: Duration = Duration::from_secs(2);
const EXIT_LIMIT: Duration = Duration::from_secs(10);
const TURN_LIMIT: Duration = Duration::from_secs(120);
const POLL: Duration = Duration::from_millis(50);
const EMPTY_MCP: &str = r#"{"mcpServers":{}}"#;
const REPLY_PROMPT: &str = "Reply with the single word: ok";
const CANCEL_PROMPT: &str = "Write the numbers from 1 to 500, one per line, with no other text.";

struct Wire {
    stdin: Stdin,
    lines: mpsc::Receiver<String>,
}

fn send(stdin: &Stdin, line: &str) -> Result<(), String> {
    let mut guard = locked(stdin);
    let stdin = guard.as_mut().ok_or("claude stdin is closed")?;
    writeln!(stdin, "{line}")
        .and_then(|()| stdin.flush())
        .map_err(|error| format!("could not write to claude: {error}"))
}

fn run_turn(wire: &Wire, text: &str, plan: ClaudePlan) -> ClaudeFold {
    let mut fold = ClaudeFold::default();
    if let Err(message) = send(&wire.stdin, &user_line(text)) {
        fold.summary.stop = Err(message);
        return fold;
    }
    let deadline = Instant::now() + TURN_LIMIT;
    while !fold.ended {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let replies = match wire.lines.recv_timeout(remaining) {
            Ok(line) => fold_claude_line(&mut fold, &line, plan),
            Err(RecvTimeoutError::Timeout) => {
                fold.summary.stop = Err(format!(
                    "claude did not finish the turn within {}s",
                    TURN_LIMIT.as_secs()
                ));
                return fold;
            }
            Err(RecvTimeoutError::Disconnected) => {
                fold.summary.stop = Err("claude exited before the turn ended".to_string());
                return fold;
            }
        };
        if let Some(message) = replies
            .iter()
            .find_map(|reply| send(&wire.stdin, reply).err())
        {
            fold.summary.stop = Err(message);
            return fold;
        }
    }
    fold
}

fn live_checks(wire: &Wire, expected_session: &str, checks: &Checks, budget: Budget) {
    if !budget.allows(checks, "prompt/reply") {
        return;
    }
    let plain = run_turn(wire, REPLY_PROMPT, ClaudePlan::Plain);
    checks.add(init_check(plain.session.as_deref(), expected_session));
    checks.add(prompt_check(&plain.summary, CLAUDE_ENDED));
    if plain.summary.stop.is_err() {
        return;
    }
    if !budget.allows(checks, "permission") {
        return;
    }
    let allowed = run_turn(wire, CLAUDE_PERMISSION_PROMPT, ClaudePlan::Allow);
    checks.add(permission_check(
        &allowed.summary,
        CLAUDE_ENDED,
        &PermissionSetup::claude_headless(),
    ));
    if !budget.allows(checks, "cancel") {
        return;
    }
    let cancelled = run_turn(wire, CANCEL_PROMPT, ClaudePlan::Cancel);
    checks.add(cancel_check(&cancelled.summary, CLAUDE_INTERRUPTED));
}

fn collect_stderr(mut stderr: ChildStderr) -> Arc<Mutex<Vec<u8>>> {
    let collected = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&collected);
    thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        while let Ok(read) = stderr.read(&mut chunk) {
            if read == 0 {
                break;
            }
            locked(&sink).extend_from_slice(&chunk[..read]);
        }
    });
    collected
}

fn wait_exit(child: &mut Child) -> Result<bool, String> {
    let deadline = Instant::now() + EXIT_LIMIT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status.success()),
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL),
            Ok(None) => {
                return Err(format!(
                    "claude did not exit within {}s after stdin closed",
                    EXIT_LIMIT.as_secs()
                ))
            }
            Err(error) => return Err(format!("could not wait for claude: {error}")),
        }
    }
}

fn verify_args(model: Option<&str>, dir: &Path, session: &str) -> Vec<String> {
    let mut args = claude_headless_args(
        model,
        EMPTY_MCP,
        dir,
        PermissionMode::Ask,
        ClaudeSession::New(session),
    );
    args.extend(["--setting-sources".to_string(), "project,local".to_string()]);
    args
}

fn claude_command(
    dir: &Path,
    session: &str,
    config: &Result<AppConfig, String>,
    model: Option<&str>,
) -> Command {
    let mut command = Command::new(CLAUDE_COMMAND);
    command
        .args(verify_args(model, dir, session))
        .current_dir(dir);
    let config_dir = config.as_ref().ok().and_then(|config| {
        configured_dir(
            config.claude.config_dir.as_deref(),
            dirs::home_dir().as_deref(),
        )
    });
    if let Some(dir) = config_dir {
        command.env("CLAUDE_CONFIG_DIR", dir);
    }
    command
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
    let budget = Budget::new(live);
    let version = step(
        checks,
        "version",
        claude_probed_version(&probe),
        |version| format!("{version} (probed {})", CLAUDE_GATE.probed),
    )?;
    let scope = SpawnScope {
        dir,
        config,
        model,
        budget,
    };
    for spawn in claude_verify_spawns(live) {
        if !budget.allows(checks, spawn_name(*spawn)) {
            break;
        }
        if !run_spawn(*spawn, &scope, checks) {
            break;
        }
    }
    Some(version)
}

fn spawn_name(kind: ClaudeSpawn) -> &'static str {
    match kind {
        ClaudeSpawn::Argv => "spawn",
        ClaudeSpawn::Turns => "live spawn",
    }
}

struct SpawnScope<'a> {
    dir: &'a Path,
    config: &'a Result<AppConfig, String>,
    model: Option<&'a str>,
    budget: Budget,
}

fn run_spawn(kind: ClaudeSpawn, scope: &SpawnScope, checks: &Checks) -> bool {
    let SpawnScope {
        dir,
        config,
        model,
        budget,
    } = *scope;
    let spawned = random_bytes().map(uuid_v4).and_then(|session| {
        spawn_piped(claude_command(dir, &session, config, model))
            .map(|spawned| (session, spawned))
            .map_err(|error| format!("could not start `{CLAUDE_COMMAND}`: {error}"))
    });
    let Some((session, (mut child, stdin, stdout, stderr))) =
        step(checks, spawn_name(kind), spawned, |_| {
            format!("`{CLAUDE_COMMAND}` started with the worker argv")
        })
    else {
        return false;
    };
    let stderr = collect_stderr(stderr);
    let log = CappedLog::new(None, "verify");
    let (lines, _) = pipe_lines(stdout, &log);
    let wire = Wire {
        stdin: logged_stdin(stdin, &log),
        lines,
    };
    thread::sleep(SETTLE);
    let running = matches!(child.try_wait(), Ok(None));
    match kind {
        ClaudeSpawn::Argv => {
            locked(&wire.stdin).take();
            let exit = if running {
                wait_exit(&mut child)
            } else {
                Ok(true)
            };
            let complaint = String::from_utf8_lossy(&locked(&stderr)).into_owned();
            checks.add(argv_check(running, &complaint, exit));
        }
        ClaudeSpawn::Turns => {
            if running {
                live_checks(&wire, &session, checks, budget);
            }
            locked(&wire.stdin).take();
        }
    }
    stop_child(&mut child);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_args_carry_the_configured_model() {
        let dir = Path::new("/tmp/verify-dir");
        let with = verify_args(Some("claude-haiku-5-5"), dir, "s1");
        let at = with.iter().position(|word| word == "--model").unwrap();
        assert_eq!(with[at + 1], "claude-haiku-5-5");
        assert!(verify_args(None, dir, "s1")
            .iter()
            .all(|word| word != "--model"));
    }

    #[test]
    fn verify_args_isolate_user_settings() {
        let args = verify_args(None, Path::new("/tmp/verify-dir"), "s1");
        let at = args
            .iter()
            .position(|word| word == "--setting-sources")
            .unwrap();
        assert_eq!(args[at + 1], "project,local");
    }

    #[test]
    fn worker_args_do_not_restrict_setting_sources() {
        let args = claude_headless_args(
            None,
            EMPTY_MCP,
            Path::new("/tmp/verify-dir"),
            PermissionMode::Ask,
            ClaudeSession::New("s1"),
        );
        assert!(args.iter().all(|word| word != "--setting-sources"));
    }
}
