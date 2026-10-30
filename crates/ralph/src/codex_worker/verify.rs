use super::*;
use crate::worker_verify::{step, Checks};
use ralph_core::worker::{
    cancel_check, permission_check, prompt_check, thread_archive_params,
    verify_thread_start_params, PermissionSetup, TurnSummary, VerifiedAgents, PROBED_VERSION,
    THREAD_ARCHIVE, TURN_COMPLETED,
};

const FREE_DEADLINE: Duration = Duration::from_secs(90);
const LIVE_DEADLINE: Duration = Duration::from_secs(420);
const AGENT_DELTA: &str = "item/agentMessage/delta";
const REPLY_PROMPT: &str = "Reply with the single word: ok";
const PERMISSION_PROMPT: &str =
    "Create a file named verify.txt in the current directory containing the word ok.";
const CANCEL_PROMPT: &str = "Write the numbers from 1 to 500, one per line, with no other text.";
const ENDED: &str = "completed";
const INTERRUPTED: &str = "interrupted";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Plan {
    Plain,
    Allow,
    Cancel,
}

struct TurnRun<'a> {
    thread: &'a str,
    text: &'a str,
    plan: Plan,
    deadline: Instant,
}

fn run_turn(wire: &mut Wire, run: &TurnRun) -> TurnSummary {
    let mut summary = TurnSummary {
        stop: Err("the turn did not finish".to_string()),
        replies: 0,
        permissions: Vec::new(),
        cancel_sent: false,
    };
    let params = turn_start_params(run.thread, run.text, &Selection::default());
    let started = match wire.call(TURN_START, params, run.deadline) {
        Ok(started) => started,
        Err(message) => {
            summary.stop = Err(message);
            return summary;
        }
    };
    let turn = decode_turn_id(&started).unwrap_or_default();
    loop {
        let incoming = match wire.next(Some(run.deadline)) {
            Ok(incoming) => incoming,
            Err(stop) => {
                summary.stop = Err(stop.message());
                return summary;
            }
        };
        match incoming {
            Incoming::Notification { method, params } if method == TURN_COMPLETED => {
                summary.stop = Ok(params["turn"]["status"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string());
                return summary;
            }
            Incoming::Notification { method, .. } if method == AGENT_DELTA => {
                summary.replies += 1;
                if run.plan == Plan::Cancel && !summary.cancel_sent {
                    let (_, line) = wire
                        .rpc
                        .request(TURN_INTERRUPT, turn_interrupt_params(run.thread, &turn));
                    summary.cancel_sent = wire.send_line(&line).is_ok();
                }
            }
            Incoming::ServerRequest { id, method, params } => {
                let approval = classify(&method, &params);
                let accept = approval
                    .as_ref()
                    .filter(|_| run.plan == Plan::Allow)
                    .and_then(yolo_option);
                let sent = match (&approval, accept) {
                    (Some(approval), Some(option)) => wire.send_line(&answer_server_request(
                        &id,
                        &response_body(approval.kind, &option.option_id),
                    )),
                    _ => wire.send_line(&reject_server_request(&id, &method)),
                };
                summary
                    .permissions
                    .extend(approval.map(|approval| approval.content));
                if let Err(stop) = sent {
                    summary.stop = Err(stop.message());
                    return summary;
                }
            }
            _ => {}
        }
    }
}

fn live_checks(wire: &mut Wire, thread: &str, checks: &Checks, deadline: Instant) {
    let run = |plan, text| TurnRun {
        thread,
        text,
        plan,
        deadline,
    };
    checks.add(prompt_check(
        &run_turn(wire, &run(Plan::Plain, REPLY_PROMPT)),
        ENDED,
    ));
    checks.add(permission_check(
        &run_turn(wire, &run(Plan::Allow, PERMISSION_PROMPT)),
        ENDED,
        &PermissionSetup::codex(),
    ));
    checks.add(cancel_check(
        &run_turn(wire, &run(Plan::Cancel, CANCEL_PROMPT)),
        INTERRUPTED,
    ));
}

fn identify(wire: &mut Wire, checks: &Checks, deadline: Instant) -> Option<String> {
    let init = step(
        checks,
        "initialize",
        wire.call(
            INITIALIZE,
            initialize_params(env!("CARGO_PKG_VERSION")),
            deadline,
        ),
        |_| "app-server answered".to_string(),
    )?;
    step(
        checks,
        "initialized",
        wire.send_line(&Rpc::notification(INITIALIZED))
            .map_err(Stop::message),
        |_| "initialized sent".to_string(),
    )?;
    let version = decode_version(&init);
    let checked = version
        .as_deref()
        .ok_or_else(|| "could not read the codex version from its initialize reply".to_string())
        .and_then(|version| check_version(version, &VerifiedAgents::default()));
    step(checks, "version", checked, |_| {
        format!(
            "{} (probed {PROBED_VERSION})",
            version.as_deref().unwrap_or_default()
        )
    })?;
    version
}

fn exercise(wire: &mut Wire, live: bool, dir: &Path, checks: &Checks, deadline: Instant) {
    let account = wire
        .call(ACCOUNT_READ, json!({}), deadline)
        .and_then(|account| check_login(&account).map(|()| account));
    if step(checks, "login", account, |_| {
        "account/read reports a usable account".to_string()
    })
    .is_none()
    {
        return;
    }
    let models = list_models(wire, deadline).and_then(|listed| match model_ids(&listed).len() {
        0 => Err("codex lists no models".to_string()),
        count => Ok(count),
    });
    if step(checks, "model/list", models, |count| {
        format!("{count} models")
    })
    .is_none()
    {
        return;
    }
    let started = wire
        .call(THREAD_START, verify_thread_start_params(dir), deadline)
        .and_then(|started| {
            decode_thread(&started).ok_or_else(|| "codex did not return a thread id".to_string())
        });
    let Some(thread) = step(checks, "thread/start", started, |thread| {
        format!("thread {}", thread.id)
    }) else {
        return;
    };
    if live {
        live_checks(wire, &thread.id, checks, deadline);
    }
    step(
        checks,
        "thread/archive",
        wire.call(THREAD_ARCHIVE, thread_archive_params(&thread.id), deadline),
        |_| "thread archived".to_string(),
    );
}

pub(crate) fn verify(live: bool, dir: &Path, checks: &Checks) -> Option<String> {
    let mut command = Command::new(CODEX_COMMAND);
    command
        .args(["app-server", "--listen", "stdio://"])
        .current_dir(dir);
    let spawned = spawn_piped(command)
        .map_err(|error| format!("could not start `{CODEX_COMMAND} app-server`: {error}"));
    let (mut child, stdin, stdout, stderr) = step(checks, "spawn", spawned, |_| {
        format!("`{CODEX_COMMAND} app-server` started")
    })?;
    let log = CappedLog::new(None, "verify");
    drain_stderr(stderr, Arc::clone(&log));
    let (lines, _) = pipe_lines(stdout, &log);
    let stdin = logged_stdin(stdin, &log);
    let mut wire = Wire {
        rpc: Rpc::default(),
        stdin: Arc::clone(&stdin),
        lines,
        startup: McpStartup::default(),
        heartbeat: Heartbeat::new(dir.join(".verify-activity"), dir.join(".verify-gone")),
        closing: Arc::new(AtomicBool::new(false)),
    };
    let deadline = Instant::now() + if live { LIVE_DEADLINE } else { FREE_DEADLINE };
    let version = identify(&mut wire, checks, deadline);
    if ralph_core::worker::spec_passed(&checks.results(), version.as_deref()) {
        exercise(&mut wire, live, dir, checks, deadline);
    }
    locked(&stdin).take();
    stop_child(&mut child);
    version
}
