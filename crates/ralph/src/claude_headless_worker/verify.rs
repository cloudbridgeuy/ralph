use super::{random_bytes, Arc, Command, Duration, Mutex};
use crate::acp_log::CappedLog;
use crate::codex_worker::pipes::{logged_stdin, pipe_lines};
use crate::codex_worker::Stdin;
use crate::config::AppConfig;
use crate::subprocess::timed_output;
use crate::transport::stdio::{spawn_piped, stop_child};
use crate::watches::locked;
use crate::worker_verify::{step, version_probe, Checks};
use ralph_core::claude_leftovers::configured_dir;
use ralph_core::stream::user_line;
use ralph_core::worker::{
    argv_check, cancel_check, claude_headless_args, claude_probed_version, fold_claude_line,
    init_check, permission_check, prompt_check, uuid_v4, ClaudeFold, ClaudePlan, ClaudeSession,
    PermissionMode, PermissionSetup, CLAUDE_COMMAND, CLAUDE_ENDED, CLAUDE_GATE, CLAUDE_INTERRUPTED,
    VERSION_PROBE_LIMIT,
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
const PERMISSION_PROMPT: &str = "Run the shell command `date +%Y` and reply with its output.";
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

fn live_checks(wire: &Wire, expected_session: &str, checks: &Checks) {
    let plain = run_turn(wire, REPLY_PROMPT, ClaudePlan::Plain);
    checks.add(init_check(plain.session.as_deref(), expected_session));
    checks.add(prompt_check(&plain.summary, CLAUDE_ENDED));
    if plain.summary.stop.is_err() {
        return;
    }
    let allowed = run_turn(wire, PERMISSION_PROMPT, ClaudePlan::Allow);
    checks.add(permission_check(
        &allowed.summary,
        CLAUDE_ENDED,
        &PermissionSetup::claude_headless(),
    ));
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

fn claude_command(dir: &Path, session: &str) -> Command {
    let args = claude_headless_args(
        None,
        EMPTY_MCP,
        dir,
        PermissionMode::Ask,
        ClaudeSession::New(session),
    );
    let mut command = Command::new(CLAUDE_COMMAND);
    command.args(args).current_dir(dir);
    let config_dir = AppConfig::load().ok().and_then(|config| {
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

pub(crate) fn verify(live: bool, dir: &Path, checks: &Checks) -> Option<String> {
    let probe = version_probe(timed_output(
        Command::new(CLAUDE_COMMAND).arg("--version"),
        VERSION_PROBE_LIMIT,
    ));
    let version = step(
        checks,
        "version",
        claude_probed_version(&probe),
        |version| format!("{version} (probed {})", CLAUDE_GATE.probed),
    )?;
    let spawned = random_bytes().map(uuid_v4).and_then(|session| {
        spawn_piped(claude_command(dir, &session))
            .map(|spawned| (session, spawned))
            .map_err(|error| format!("could not start `{CLAUDE_COMMAND}`: {error}"))
    });
    let Some((session, (mut child, stdin, stdout, stderr))) =
        step(checks, "spawn", spawned, |_| {
            format!("`{CLAUDE_COMMAND}` started with the worker argv")
        })
    else {
        return Some(version);
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
    if running && live {
        live_checks(&wire, &session, checks);
    }
    locked(&wire.stdin).take();
    let exit = if running {
        wait_exit(&mut child)
    } else {
        Ok(true)
    };
    let complaint = String::from_utf8_lossy(&locked(&stderr)).into_owned();
    checks.add(argv_check(running, &complaint, exit));
    stop_child(&mut child);
    Some(version)
}
