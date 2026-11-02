use super::{
    free_port, open_log, post_prompt, random_password, read_lines, Feed, Server, EVENTS_PATH,
    MESSAGE_LIMIT,
};
use crate::subprocess::timed_output;
use crate::transport::http::HttpSse;
use crate::worker_verify::{step, version_probe, Checks};
use ralph_core::worker::{
    cancel_check, fold_opencode_event, interrupt_idle_check, messages_check,
    opencode_probed_version, opencode_serve_command, opencode_server_log, opencode_session_id,
    permission_check, permission_reply_body, permission_reply_path, prompt_check, reply_count,
    session_check, spawn_failure, start_failure, verify_session_body, CheckResult, OpencodeAction,
    OpencodeFold, OpencodePlan, PermissionSetup, Verdict, DEFAULT_OPENCODE_MODEL, OPENCODE_COMMAND,
    OPENCODE_ENDED, OPENCODE_INTERRUPTED, OPENCODE_PASSWORD_ENV, VERSION_PROBE_LIMIT,
};
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

const TURN_LIMIT: Duration = Duration::from_secs(120);
const MAIN_SESSION: &str = "verify";
const ASK_SESSION: &str = "verifyask";
const REPLY_PROMPT: &str = "Reply with the single word: ok";
const PERMISSION_PROMPT: &str = "Run the shell command `date +%Y` and reply with its output.";
const CANCEL_PROMPT: &str = "Write the numbers from 1 to 500, one per line, with no other text.";

fn launch(dir: &Path) -> Result<Server, String> {
    let port = free_port()?;
    let password = random_password()?;
    let log = open_log(dir)?;
    let command = opencode_serve_command(port);
    let (program, args) = command.split_first().ok_or("empty opencode command")?;
    let child = Command::new(program)
        .args(args)
        .env(OPENCODE_PASSWORD_ENV, &password)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log)
        .process_group(0)
        .spawn()
        .map_err(|error| spawn_failure(program, &error))?;
    let mut server = Server {
        child,
        api: HttpSse::new(port, &password),
        tree: Vec::new(),
        record: dir.join("server-record"),
    };
    match server.wait_healthy(&AtomicBool::new(false)) {
        Ok(()) => Ok(server),
        Err(message) => {
            server.stop();
            let log = fs::read_to_string(opencode_server_log(dir)).unwrap_or_default();
            Err(start_failure(&message, &log))
        }
    }
}

fn session_path(id: &str) -> String {
    format!("/api/session/{id}")
}

fn messages_path(id: &str) -> String {
    format!("{}/message?limit={MESSAGE_LIMIT}", session_path(id))
}

fn interrupt(api: &HttpSse, id: &str) -> Result<String, String> {
    api.post(&format!("{}/interrupt", session_path(id)), "")
}

fn create_session(api: &HttpSse, dir: &Path, id: &str, ask: bool) -> Result<String, String> {
    let body = verify_session_body(id, &dir.to_string_lossy(), DEFAULT_OPENCODE_MODEL, ask)?;
    api.post_retrying("/api/session", &body.to_string())
}

fn read_session(api: &HttpSse, id: &str) -> Result<String, String> {
    let session = api.get(&session_path(id))?;
    if !session.contains(id) {
        return Err(format!("the session answer did not name {id}: {session}"));
    }
    api.get(&messages_path(id))
}

fn judged(
    checks: &Checks,
    name: &'static str,
    outcome: Result<String, String>,
    judge: impl FnOnce(&str) -> CheckResult,
) -> bool {
    let result = outcome.map_or_else(
        |message| CheckResult::fail(name, message),
        |body| judge(&body),
    );
    let passed = result.verdict == Verdict::Pass;
    checks.add(result);
    passed
}

fn act(api: &HttpSse, id: &str, action: OpencodeAction) -> Result<(), String> {
    match action {
        OpencodeAction::Answer {
            request_id,
            decision,
        } => api
            .post_retrying(
                &permission_reply_path(id, &request_id),
                &permission_reply_body(decision).to_string(),
            )
            .map(|_| ()),
        OpencodeAction::Interrupt => interrupt(api, id).map(|_| ()),
    }
}

fn run_turn(
    api: &HttpSse,
    feed: &Receiver<Feed>,
    id: &str,
    text: &str,
    plan: OpencodePlan,
) -> OpencodeFold {
    let mut fold = OpencodeFold::default();
    if let Err(message) = post_prompt(api, id, text) {
        fold.summary.stop = Err(message);
        return fold;
    }
    let deadline = Instant::now() + TURN_LIMIT;
    while !fold.ended {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match feed.recv_timeout(remaining) {
            Ok(Feed::Line(line)) => {
                let failure = fold_opencode_event(&mut fold, id, &line, plan)
                    .into_iter()
                    .find_map(|action| act(api, id, action).err());
                if let Some(message) = failure {
                    fold.summary.stop = Err(message);
                    return fold;
                }
            }
            Ok(Feed::Ended) | Err(RecvTimeoutError::Disconnected) => {
                fold.summary.stop = Err("the event stream ended before the turn ended".to_string());
                return fold;
            }
            Err(RecvTimeoutError::Timeout) => {
                fold.summary.stop = Err(format!(
                    "opencode did not finish the turn within {}s",
                    TURN_LIMIT.as_secs()
                ));
                return fold;
            }
        }
    }
    fold.summary.replies = api
        .get(&messages_path(id))
        .map_or(0, |body| reply_count(&body));
    fold
}

fn live_checks(api: &HttpSse, feed: &Receiver<Feed>, dir: &Path, id: &str, checks: &Checks) {
    let plain = run_turn(api, feed, id, REPLY_PROMPT, OpencodePlan::Plain);
    checks.add(prompt_check(&plain.summary, OPENCODE_ENDED));
    if plain.summary.stop.is_err() {
        return;
    }
    let ask_id = opencode_session_id(ASK_SESSION);
    match create_session(api, dir, &ask_id, true) {
        Ok(_) => {
            let allowed = run_turn(api, feed, &ask_id, PERMISSION_PROMPT, OpencodePlan::Allow);
            checks.add(permission_check(
                &allowed.summary,
                OPENCODE_ENDED,
                &PermissionSetup::opencode(),
            ));
            api.delete(&session_path(&ask_id)).ok();
        }
        Err(message) => checks.add(CheckResult::fail("permission", message)),
    }
    let cancelled = run_turn(api, feed, id, CANCEL_PROMPT, OpencodePlan::Cancel);
    checks.add(cancel_check(&cancelled.summary, OPENCODE_INTERRUPTED));
}

fn run_checks(api: &HttpSse, dir: &Path, live: bool, checks: &Checks) {
    let feed = step(checks, "events", api.events(EVENTS_PATH), |_| {
        format!("{EVENTS_PATH} connected")
    })
    .map(|reader| {
        let (sender, feed) = mpsc::channel();
        thread::spawn(move || read_lines(reader, &sender));
        feed
    });
    let id = opencode_session_id(MAIN_SESSION);
    let created = judged(
        checks,
        "session/create",
        create_session(api, dir, &id, false),
        |body| session_check("session/create", body, &id),
    );
    if !created {
        return;
    }
    judged(
        checks,
        "session/read",
        read_session(api, &id),
        messages_check,
    );
    judged(
        checks,
        "interrupt/idle",
        interrupt(api, &id),
        interrupt_idle_check,
    );
    if let Some(feed) = feed.as_ref().filter(|_| live) {
        live_checks(api, feed, dir, &id, checks);
    }
    step(
        checks,
        "session/delete",
        api.delete(&session_path(&id)),
        |_| format!("session {id} deleted"),
    );
}

pub(crate) fn verify(live: bool, dir: &Path, checks: &Checks) -> Option<String> {
    let probe = version_probe(timed_output(
        Command::new(OPENCODE_COMMAND).arg("--version"),
        VERSION_PROBE_LIMIT,
    ));
    let version = step(
        checks,
        "version",
        opencode_probed_version(&probe),
        |version| version.clone(),
    )?;
    let Some(mut server) = step(checks, "spawn", launch(dir), |server| {
        format!(
            "`opencode serve` answered {} ({})",
            super::HEALTH_PATH,
            server.label()
        )
    }) else {
        return Some(version);
    };
    run_checks(&server.api, dir, live, checks);
    server.snapshot();
    server.stop();
    Some(version)
}
