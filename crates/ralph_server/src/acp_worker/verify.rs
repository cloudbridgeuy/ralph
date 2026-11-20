use super::*;
use crate::cursor_sweep::sweep_worker_server;
use crate::cursor_usage::{record_cursor_verify, CursorTurn};
use crate::worker_verify::{step, Checks};
use agent_client_protocol::ByteStreams;
use ralph_core::worker::{
    cursor_verify_leftovers, cursor_verify_project, decode_print_line, print_args, print_model,
    print_verify, probed_version, stderr_tail, CheckResult, PrintArgs, PrintEvent, VersionProbe,
    ACP_PROTOCOL_VERSION, AUTH_REQUIRED_CODE, CURSOR_HOOKS_FILE, CURSOR_VERIFY_HOOKS,
    STDERR_TAIL_CAP,
};
use std::io::{BufRead, BufReader, Read, Write};

const FREE_DEADLINE: Duration = Duration::from_secs(90);
const PRINT_DEADLINE: Duration = Duration::from_secs(120);
const KILL_GRACE: Duration = Duration::from_millis(200);
const REPLY_PROMPT: &str = "Reply with the single word: ok";

async fn open(
    connection: &ConnectionTo<Agent>,
    dir: &Path,
    checks: &Checks,
) -> Option<(InitializeResponse, SessionId)> {
    let init = step(
        checks,
        "initialize",
        match initialize(connection, &CURSOR_AGENT).await {
            Ok(checked) => checked,
            Err(error) => Err(error.message),
        },
        |_| format!("ACP protocol {ACP_PROTOCOL_VERSION}"),
    )?;
    let created = match authenticate(connection, &CURSOR_AGENT, &init).await {
        Ok(()) => {
            connection
                .send_request(NewSessionRequest::new(dir.to_path_buf()))
                .block_task()
                .await
        }
        Err(error) => Err(error),
    };
    let logged_out = matches!(&created, Err(error) if i32::from(error.code) == AUTH_REQUIRED_CODE);
    let created = created
        .map_err(|error| session_error(&CURSOR_AGENT, i32::from(error.code), &error.message));
    if logged_out {
        checks.add(CheckResult::fail(
            "login",
            created.as_ref().err().cloned().unwrap_or_default(),
        ));
        return None;
    }
    checks.add(CheckResult::pass(
        "login",
        "authenticate and session/new raised no auth error",
    ));
    let created = step(checks, "session/new", created, |created| {
        format!("session {}", created.session_id)
    })?;
    Some((init, created.session_id))
}

fn check_load_capability(init: &InitializeResponse) -> Result<String, String> {
    if init.agent_capabilities.load_session {
        Ok("loadSession capability advertised".to_string())
    } else {
        Err("the agent does not advertise the loadSession capability".to_string())
    }
}

async fn run_checks(
    connection: ConnectionTo<Agent>,
    dir: PathBuf,
    checks: Checks,
) -> Result<(), agent_client_protocol::Error> {
    let Some((init, _)) = open(&connection, &dir, &checks).await else {
        return Ok(());
    };
    step(
        &checks,
        "session/load",
        check_load_capability(&init),
        Clone::clone,
    );
    Ok(())
}

pub(super) fn agent_version_probe(agent: &AcpAgent) -> VersionProbe {
    crate::version_drift::probe(agent.command)
}

fn watchdog(group: u32, deadline: Duration) -> mpsc::Sender<()> {
    let (done, waiting) = mpsc::channel();
    thread::spawn(move || {
        if let Err(mpsc::RecvTimeoutError::Timeout) = waiting.recv_timeout(deadline) {
            stop_groups(&[group], KILL_GRACE, || false);
        }
    });
    done
}

pub(crate) fn verify(live: Option<&Path>, dir: &Path, checks: &Checks) -> Option<String> {
    let agent = CURSOR_AGENT;
    let probe = agent_version_probe(&agent);
    let version = Some(step(
        checks,
        "version",
        probed_version(&agent, &probe).map(str::to_string),
        |version| format!("{version} (tested {})", agent.tested_version),
    )?);
    let Some(home) = dirs::home_dir() else {
        checks.add(CheckResult::fail(
            "hooks",
            "could not find the home directory",
        ));
        return version;
    };
    let project = cursor_verify_project(&home, dir);
    if project.exists() {
        checks.add(CheckResult::fail(
            "hooks",
            format!(
                "{} already exists; verify writes its hook only into a Cursor project folder it creates",
                project.display()
            ),
        ));
        return version;
    }
    let leftovers: Vec<PathBuf> = fs::canonicalize(dir)
        .ok()
        .map(|real| cursor_verify_leftovers(&home, dir, &real))
        .unwrap_or_default()
        .into_iter()
        .filter(|leftover| !leftover.exists())
        .collect();
    let hooks = project.join(CURSOR_HOOKS_FILE);
    let written = hooks
        .parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| fs::write(&hooks, CURSOR_VERIFY_HOOKS))
        .map_err(|error| format!("could not write {}: {error}", hooks.display()));
    if step(checks, "hooks", written, |()| {
        format!("wrote {}", hooks.display())
    })
    .is_some()
    {
        exercise(dir, checks);
        if let Some(ledger) = live {
            print_turn(ledger, dir, checks);
        }
    }
    let removed = fs::remove_dir_all(&project)
        .map_err(|error| format!("could not remove {}: {error}", project.display()));
    step(checks, "hooks cleanup", removed, |()| {
        format!("removed {}", project.display())
    });
    for leftover in leftovers.iter().filter(|leftover| leftover.exists()) {
        let removed = fs::remove_dir_all(leftover)
            .map_err(|error| format!("could not remove {}: {error}", leftover.display()));
        step(checks, "cursor cleanup", removed, |()| {
            format!("removed {}", leftover.display())
        });
    }
    version
}

fn exercise(dir: &Path, checks: &Checks) {
    let agent = CURSOR_AGENT;
    let mut command = Command::new(agent.command);
    command.arg("acp").current_dir(dir);
    let spawned = spawn_piped(command)
        .map_err(|error| format!("could not start `{} acp`: {error}", agent.command));
    let Some((mut child, stdin, stdout, stderr)) = step(checks, "spawn", spawned, |_| {
        format!("`{} acp` started", agent.command)
    }) else {
        return;
    };
    let first_row = checks.len();
    let captured = Arc::new(Mutex::new(Vec::new()));
    let reader = {
        let captured = Arc::clone(&captured);
        thread::spawn(move || {
            let mut stderr = stderr;
            let mut buffer = Vec::new();
            stderr.read_to_end(&mut buffer).ok();
            *locked(&captured) = buffer;
        })
    };
    let done = watchdog(child.id(), FREE_DEADLINE);
    let outcome = block_on(Client.builder().connect_with(
        ByteStreams::new(
            blocking::Unblock::new(stdin),
            blocking::Unblock::new(stdout),
        ),
        {
            let (dir, checks) = (dir.to_path_buf(), checks.clone());
            async move |connection: ConnectionTo<Agent>| run_checks(connection, dir, checks).await
        },
    ));
    done.send(()).ok();
    stop_child(&mut child);
    if let Err(error) = outcome {
        checks.add(CheckResult::fail(
            "connection",
            format!("{} agent connection failed: {error}", agent.name),
        ));
    }
    reader.join().ok();
    let tail = stderr_tail(&locked(&captured), STDERR_TAIL_CAP);
    checks.append_stderr_since(first_row, &tail);
}

fn print_lines(dir: &Path, model: &str) -> Result<Vec<String>, String> {
    let args = print_args(&PrintArgs {
        workspace: dir,
        model,
        resume: None,
    });
    let mut command = Command::new(CURSOR_AGENT.command);
    command.args(args).current_dir(dir);
    let (mut child, mut stdin, stdout, stderr) = spawn_piped(command)
        .map_err(|error| format!("could not start `{} -p`: {error}", CURSOR_AGENT.command))?;
    drain_stderr(stderr, CappedLog::new(None, "verify"));
    if let Err(error) = stdin.write_all(REPLY_PROMPT.as_bytes()) {
        stop_child(&mut child);
        return Err(format!("could not write the prompt: {error}"));
    }
    drop(stdin);
    let (sender, incoming) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let deadline = Instant::now() + PRINT_DEADLINE;
    let mut lines = Vec::new();
    while let Ok(line) = incoming.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        let ended = matches!(decode_print_line(&line, model), PrintEvent::Result { .. });
        lines.push(line);
        if ended {
            break;
        }
    }
    stop_child(&mut child);
    Ok(lines)
}

fn print_turn(ledger: &Path, dir: &Path, checks: &Checks) {
    let model = print_model(None);
    let lines = match print_lines(dir, model) {
        Ok(lines) => lines,
        Err(message) => {
            checks.add(CheckResult::fail("print prompt", message));
            return;
        }
    };
    if let Some(home) = dirs::home_dir() {
        let real = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
        sweep_worker_server(&home.join(".cursor/projects"), &real);
    }
    let verified = print_verify(&lines, model);
    checks.add(verified.prompt);
    let recorded = match verified.usage {
        Some(usage) => {
            let turn = CursorTurn {
                worker: "verify",
                chat: None,
                store: None,
                turn_id: None,
            };
            record_cursor_verify(&turn, &usage, ledger)
                .map(|()| {
                    format!(
                        "recorded {} output tokens in the ledger",
                        usage.usage.output
                    )
                })
                .map_err(|error| format!("could not record the usage: {error}"))
        }
        None => Err("cursor reported no usage".to_string()),
    };
    step(checks, "print usage", recorded, Clone::clone);
}
