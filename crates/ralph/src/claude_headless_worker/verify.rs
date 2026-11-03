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
    acp_mcp_server, argv_check, cancel_check, claude_headless_args, claude_mcp_config,
    claude_probed_version, claude_verify_spawns, fold_claude_line, init_check, mcp_check,
    parse_reports, permission_check, prompt_check, report_check, uuid_v4, CheckResult, ClaudeFold,
    ClaudePlan, ClaudeSession, ClaudeSpawn, PermissionMode, PermissionSetup, StderrRead,
    CLAUDE_COMMAND, CLAUDE_ENDED, CLAUDE_GATE, CLAUDE_INTERRUPTED, CLAUDE_PERMISSION_PROMPT,
    CLAUDE_REPORT_PROMPT, VERSION_PROBE_LIMIT,
};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Instant;

const SETTLE: Duration = Duration::from_secs(2);
const EXIT_LIMIT: Duration = Duration::from_secs(10);
const TURN_LIMIT: Duration = Duration::from_secs(120);
const STDERR_LIMIT: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(50);
const VERIFY_ID: &str = "verify";
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

fn live_checks(wire: &Wire, expected_session: &str, dir: &Path, checks: &Checks, budget: Budget) {
    if !budget.allows(checks, "prompt/reply") {
        return;
    }
    let plain = run_turn(wire, REPLY_PROMPT, ClaudePlan::Plain);
    checks.add(init_check(plain.session.as_deref(), expected_session));
    checks.add(mcp_check(&plain));
    checks.add(prompt_check(&plain.summary, CLAUDE_ENDED));
    if plain.summary.stop.is_err() {
        return;
    }
    if !budget.allows(checks, "report") {
        return;
    }
    let mut reported = run_turn(wire, CLAUDE_REPORT_PROMPT, ClaudePlan::Plain);
    reported.mcp = plain.mcp.clone();
    checks.add(report_check(&reported, &read_inbox(dir)));
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

fn inbox_dir(dir: &Path) -> PathBuf {
    dir.join("inbox")
}

fn owner_file(dir: &Path) -> PathBuf {
    dir.join("owner")
}

fn prepare_inbox(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(inbox_dir(dir))
        .and_then(|()| fs::write(owner_file(dir), ""))
        .map_err(|error| format!("could not prepare the verify inbox: {error}"))
}

fn read_inbox(dir: &Path) -> Vec<ralph_core::worker::Report> {
    let Ok(entries) = fs::read_dir(inbox_dir(dir)) else {
        return Vec::new();
    };
    let texts: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.ends_with(".json") && !name.starts_with('.')
        })
        .filter_map(|entry| fs::read_to_string(entry.path()).ok())
        .collect();
    parse_reports(&texts)
}

struct StderrReader {
    collected: Arc<Mutex<Vec<u8>>>,
    finished: mpsc::Receiver<()>,
}

impl StderrReader {
    fn start(mut stderr: ChildStderr) -> Self {
        let collected = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&collected);
        let (done, finished) = mpsc::channel::<()>();
        thread::spawn(move || {
            let _done = done;
            let mut chunk = [0u8; 4096];
            while let Ok(read) = stderr.read(&mut chunk) {
                if read == 0 {
                    break;
                }
                locked(&sink).extend_from_slice(&chunk[..read]);
            }
        });
        Self {
            collected,
            finished,
        }
    }

    fn read(&self, limit: Duration) -> StderrRead {
        let finished = !matches!(
            self.finished.recv_timeout(limit),
            Err(RecvTimeoutError::Timeout)
        );
        let text = String::from_utf8_lossy(&locked(&self.collected)).into_owned();
        if finished {
            StderrRead::Finished(text)
        } else {
            StderrRead::Unfinished(text)
        }
    }
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

fn verify_mcp_config(exe: &Path, dir: &Path) -> String {
    let server = acp_mcp_server(exe, VERIFY_ID, &inbox_dir(dir), &owner_file(dir), None);
    claude_mcp_config(&server).to_string()
}

fn verify_args(model: Option<&str>, dir: &Path, session: &str, exe: &Path) -> Vec<String> {
    let mut args = claude_headless_args(
        model,
        &verify_mcp_config(exe, dir),
        dir,
        PermissionMode::Ask,
        ClaudeSession::New(session),
    );
    args.extend(["--setting-sources".to_string(), "project,local".to_string()]);
    args
}

fn claude_command(
    exe: &Path,
    dir: &Path,
    session: &str,
    config: &Result<AppConfig, String>,
    model: Option<&str>,
) -> Command {
    let mut command = Command::new(CLAUDE_COMMAND);
    command
        .args(verify_args(model, dir, session, exe))
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
    let budget = Budget::new(live);
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
    if let Err(message) = prepare_inbox(dir) {
        checks.add(CheckResult::fail("spawn", message));
        return Some(version);
    }
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
        let exe = std::env::current_exe()
            .map_err(|error| format!("could not find the ralph executable: {error}"))?;
        spawn_piped(claude_command(&exe, dir, &session, config, model))
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
    let stderr = StderrReader::start(stderr);
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
            checks.add(argv_check(running, &stderr.read(STDERR_LIMIT), exit));
        }
        ClaudeSpawn::Turns => {
            if running {
                live_checks(&wire, &session, dir, checks, budget);
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

    fn exe() -> PathBuf {
        PathBuf::from("/usr/bin/ralph")
    }

    #[test]
    fn verify_args_carry_the_configured_model() {
        let dir = Path::new("/tmp/verify-dir");
        let with = verify_args(Some("claude-haiku-5-5"), dir, "s1", &exe());
        let at = with.iter().position(|word| word == "--model").unwrap();
        assert_eq!(with[at + 1], "claude-haiku-5-5");
        assert!(verify_args(None, dir, "s1", &exe())
            .iter()
            .all(|word| word != "--model"));
    }

    #[test]
    fn verify_args_load_the_ralph_worker_server_with_a_temporary_inbox() {
        let args = verify_args(None, Path::new("/tmp/verify-dir"), "s1", &exe());
        let at = args.iter().position(|word| word == "--mcp-config").unwrap();
        let config: serde_json::Value = serde_json::from_str(&args[at + 1]).unwrap();
        let server_args = config["mcpServers"]["ralph-worker"]["args"]
            .as_array()
            .unwrap();
        let words: Vec<&str> = server_args.iter().filter_map(|w| w.as_str()).collect();
        assert_eq!(words[0], "worker-mcp");
        assert!(words.windows(2).any(|pair| pair == ["--id", "verify"]));
        assert!(words
            .windows(2)
            .any(|pair| pair == ["--inbox", "/tmp/verify-dir/inbox"]));
        assert!(words
            .windows(2)
            .any(|pair| pair == ["--owner", "/tmp/verify-dir/owner"]));
    }

    #[test]
    fn verify_args_isolate_user_settings() {
        let args = verify_args(None, Path::new("/tmp/verify-dir"), "s1", &exe());
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
            "{}",
            Path::new("/tmp/verify-dir"),
            PermissionMode::Ask,
            ClaudeSession::New("s1"),
        );
        assert!(args.iter().all(|word| word != "--setting-sources"));
    }
}
