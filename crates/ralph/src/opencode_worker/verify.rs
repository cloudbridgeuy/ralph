use super::{
    free_port, open_log, post_prompt, random_password, read_lines, Feed, Server, EVENTS_PATH,
    MESSAGE_LIMIT,
};
use crate::subprocess::{register_group, timed_output};
use crate::transport::http::HttpSse;
use crate::worker_verify::{step, version_probe, Budget, Checks};
use ralph_core::worker::{
    cancel_check, delete_result, fold_opencode_event, interrupt_idle_check, messages_check,
    opencode_probed_version, opencode_serve_command, opencode_server_log, opencode_session_id,
    permission_check, permission_reply_body, permission_reply_path, prompt_check, reply_count,
    session_check, spawn_failure, start_failure, verify_session_body, CheckResult, OpencodeAction,
    OpencodeFold, OpencodePlan, PermissionSetup, Verdict, OPENCODE_COMMAND, OPENCODE_ENDED,
    OPENCODE_INTERRUPTED, OPENCODE_PASSWORD_ENV, VERSION_PROBE_LIMIT,
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
const EVENTS_LIMIT: Duration = Duration::from_secs(10);
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
    register_group(child.id());
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

#[derive(Clone, Copy)]
struct Scope<'a> {
    dir: &'a Path,
    model: &'a str,
    budget: Budget,
}

fn session_json(scope: Scope, id: &str, ask: bool) -> Result<String, String> {
    verify_session_body(id, &scope.dir.to_string_lossy(), scope.model, ask)
        .map(|body| body.to_string())
}

fn create_session(api: &HttpSse, scope: Scope, id: &str, ask: bool) -> Result<String, String> {
    api.post_retrying("/api/session", &session_json(scope, id, ask)?)
}

fn read_session(api: &HttpSse, id: &str) -> Result<String, String> {
    let session = api.get(&session_path(id))?;
    let named = session_check("session/read", &session, id);
    if named.verdict != Verdict::Pass {
        return Err(named.detail);
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

fn live_checks(api: &HttpSse, feed: &Receiver<Feed>, scope: Scope, id: &str, checks: &Checks) {
    if !scope.budget.allows(checks, "prompt/reply") {
        return;
    }
    let plain = run_turn(api, feed, id, REPLY_PROMPT, OpencodePlan::Plain);
    checks.add(prompt_check(&plain.summary, OPENCODE_ENDED));
    if plain.summary.stop.is_err() {
        return;
    }
    if !scope.budget.allows(checks, "permission") {
        return;
    }
    let ask_id = opencode_session_id(ASK_SESSION);
    match create_session(api, scope, &ask_id, true) {
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
    if !scope.budget.allows(checks, "cancel") {
        return;
    }
    let cancelled = run_turn(api, feed, id, CANCEL_PROMPT, OpencodePlan::Cancel);
    checks.add(cancel_check(&cancelled.summary, OPENCODE_INTERRUPTED));
}

fn open_events(api: &HttpSse, path: &str, limit: Duration) -> Result<Receiver<Feed>, String> {
    let (sender, feed) = mpsc::channel();
    let (opened, answer) = mpsc::channel();
    let api = api.clone();
    let owned = path.to_string();
    thread::spawn(move || match api.events(&owned) {
        Ok(reader) => {
            opened.send(Ok(())).ok();
            read_lines(reader, &sender);
        }
        Err(message) => {
            opened.send(Err(message)).ok();
        }
    });
    match answer.recv_timeout(limit) {
        Ok(Ok(())) => Ok(feed),
        Ok(Err(message)) => Err(message),
        Err(RecvTimeoutError::Timeout) => Err(format!("{path} did not answer within {limit:?}")),
        Err(RecvTimeoutError::Disconnected) => {
            Err(format!("the {path} request ended without an answer"))
        }
    }
}

fn run_checks(api: &HttpSse, scope: Scope, live: bool, checks: &Checks) {
    let budget = scope.budget;
    if !budget.allows(checks, "events") {
        return;
    }
    let feed = step(
        checks,
        "events",
        open_events(api, EVENTS_PATH, EVENTS_LIMIT),
        |_| format!("{EVENTS_PATH} connected"),
    );
    if !budget.allows(checks, "session/create") {
        return;
    }
    let id = opencode_session_id(MAIN_SESSION);
    let created = judged(
        checks,
        "session/create",
        create_session(api, scope, &id, false),
        |body| session_check("session/create", body, &id),
    );
    if !created || !budget.allows(checks, "session/read") {
        return;
    }
    judged(
        checks,
        "session/read",
        read_session(api, &id),
        messages_check,
    );
    if !budget.allows(checks, "interrupt/idle") {
        return;
    }
    judged(
        checks,
        "interrupt/idle",
        interrupt(api, &id),
        interrupt_idle_check,
    );
    if let Some(feed) = feed.as_ref().filter(|_| live) {
        live_checks(api, feed, scope, &id, checks);
    }
    checks.add(delete_result(
        budget.expired("session/delete"),
        api.delete(&session_path(&id)).map(drop),
        &id,
    ));
}

pub(crate) fn verify(live: bool, dir: &Path, checks: &Checks, model: &str) -> Option<String> {
    let budget = Budget::new(live);
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
    if !budget.allows(checks, "spawn") {
        return Some(version);
    }
    let Some(mut server) = step(checks, "spawn", launch(dir), |server| {
        format!(
            "`opencode serve` answered {} ({})",
            super::HEALTH_PATH,
            server.label()
        )
    }) else {
        return Some(version);
    };
    run_checks(&server.api, Scope { dir, model, budget }, live, checks);
    server.snapshot();
    server.stop();
    Some(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_body_carries_the_scope_model() {
        let scope = Scope {
            dir: Path::new("/tmp/verify-dir"),
            model: "anthropic/claude-haiku-5-5",
            budget: Budget::new(false),
        };
        let body = session_json(scope, "verify", false).unwrap();
        assert!(body.contains("claude-haiku-5-5"), "{body}");
    }

    #[test]
    fn a_server_that_never_answers_the_events_request_fails_by_the_deadline() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (release, held) = mpsc::channel::<()>();
        thread::spawn(move || {
            let accepted = listener.accept();
            held.recv().ok();
            drop(accepted);
        });
        let (done, outcome) = mpsc::channel();
        thread::spawn(move || {
            let api = HttpSse::new(port, "secret");
            let started = Instant::now();
            let result = open_events(&api, EVENTS_PATH, Duration::from_millis(300));
            done.send((result.err(), started.elapsed())).ok();
        });
        let verdict = outcome.recv_timeout(Duration::from_secs(3));
        release.send(()).ok();
        let (error, elapsed) = verdict.expect("open_events did not return by its deadline");
        assert!(error.is_some());
        assert!(elapsed < Duration::from_secs(2), "{elapsed:?}");
    }
}
