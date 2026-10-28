use super::*;
use crate::worker_verify::{step, Checks};
use ralph_core::worker::{
    cancel_check, parse_agent_version, permission_check, prompt_check, BlockedContent, CheckResult,
    TurnSummary, ACP_PROTOCOL_VERSION, AUTH_REQUIRED_CODE, CURSOR_VERIFY_HOOKS,
};
use std::sync::atomic::AtomicUsize;

const FREE_DEADLINE: Duration = Duration::from_secs(90);
const LIVE_DEADLINE: Duration = Duration::from_secs(420);
const KILL_GRACE: Duration = Duration::from_millis(200);
const REPLY_PROMPT: &str = "Reply with the single word: ok";
const PERMISSION_PROMPT: &str = "Run the shell command `echo ok` and reply with its output.";
const PERMISSION_SETUP: &str =
    "the temporary directory's .cursor/hooks.json asks before every shell command";
const CANCEL_PROMPT: &str = "Write the numbers from 1 to 500, one per line, with no other text.";
const ENDED: &str = "end_turn";
const CANCELLED: &str = "cancelled";

struct Probe {
    allowing: Arc<AtomicBool>,
    permissions: Arc<Mutex<Vec<BlockedContent>>>,
    replies: Arc<AtomicUsize>,
    updates: queue::UnboundedReceiver<()>,
}

struct Session<'a> {
    connection: &'a ConnectionTo<Agent>,
    id: SessionId,
    probe: Probe,
}

enum Plan {
    Plain,
    Allow,
    Cancel,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn permission_reply(
    request: &RequestPermissionRequest,
    allowing: bool,
    seen: &Mutex<Vec<BlockedContent>>,
) -> RequestPermissionResponse {
    let content = permission_content(&serde_json::to_value(request).unwrap_or_default());
    let allow = content
        .options
        .iter()
        .find(|option| option.kind == "allow_once")
        .filter(|_| allowing)
        .map(|option| option.option_id.clone());
    locked(seen).push(content);
    RequestPermissionResponse::new(match allow {
        Some(option_id) => {
            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option_id))
        }
        None => RequestPermissionOutcome::Cancelled,
    })
}

impl Session<'_> {
    async fn turn(&mut self, text: &str, plan: Plan) -> TurnSummary {
        while self.probe.updates.try_recv().is_ok() {}
        self.probe
            .allowing
            .store(matches!(plan, Plan::Allow), Ordering::SeqCst);
        let replies_before = self.probe.replies.load(Ordering::SeqCst);
        let permissions_before = locked(&self.probe.permissions).len();
        let mut sent = Box::pin(
            self.connection
                .send_request(PromptRequest::new(self.id.clone(), vec![text.into()]))
                .block_task(),
        )
        .fuse();
        let mut cancel_sent = false;
        let result = loop {
            select! {
                result = sent => break result,
                _ = self.probe.updates.select_next_some() => {
                    if matches!(plan, Plan::Cancel) && !cancel_sent {
                        cancel_sent = true;
                        self.connection
                            .send_notification(CancelNotification::new(self.id.clone()))
                            .ok();
                    }
                }
            }
        };
        self.probe.allowing.store(false, Ordering::SeqCst);
        TurnSummary {
            stop: result
                .map(|done| {
                    serde_json::to_value(done.stop_reason)
                        .ok()
                        .and_then(|value| value.as_str().map(str::to_string))
                        .unwrap_or_default()
                })
                .map_err(|error| error.message),
            replies: self.probe.replies.load(Ordering::SeqCst) - replies_before,
            permissions: locked(&self.probe.permissions).split_off(permissions_before),
            cancel_sent,
        }
    }
}

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
    let created = connection
        .send_request(NewSessionRequest::new(dir.to_path_buf()))
        .block_task()
        .await;
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
        "session/new raised no auth error",
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

async fn reload_session(
    connection: &ConnectionTo<Agent>,
    session: &SessionId,
    dir: &Path,
) -> Result<String, String> {
    connection
        .send_request(LoadSessionRequest::new(session.clone(), dir.to_path_buf()))
        .block_task()
        .await
        .map(|_| "the prompted session loaded again".to_string())
        .map_err(|error| error.message)
}

async fn run_checks(
    connection: ConnectionTo<Agent>,
    dir: PathBuf,
    live: bool,
    checks: Checks,
    probe: Probe,
) -> Result<(), agent_client_protocol::Error> {
    let Some((init, id)) = open(&connection, &dir, &checks).await else {
        return Ok(());
    };
    step(
        &checks,
        "session/load",
        check_load_capability(&init),
        Clone::clone,
    );
    if live {
        let mut session = Session {
            connection: &connection,
            id,
            probe,
        };
        checks.add(prompt_check(
            &session.turn(REPLY_PROMPT, Plan::Plain).await,
            ENDED,
        ));
        let reloaded = reload_session(&connection, &session.id, &dir).await;
        step(&checks, "session/reload", reloaded, Clone::clone);
        checks.add(permission_check(
            &session.turn(PERMISSION_PROMPT, Plan::Allow).await,
            ENDED,
            PERMISSION_SETUP,
        ));
        checks.add(cancel_check(
            &session.turn(CANCEL_PROMPT, Plan::Cancel).await,
            CANCELLED,
        ));
    }
    Ok(())
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

pub(crate) fn verify(live: bool, dir: &Path, checks: &Checks) -> Option<String> {
    let agent = CURSOR_AGENT;
    let output = agent_version_output(&agent);
    let version = output
        .as_deref()
        .and_then(parse_agent_version)
        .map(str::to_string);
    step(
        checks,
        "version",
        version.clone().ok_or_else(|| {
            format!(
                "could not read the version from `{} --version`",
                agent.command
            )
        }),
        |version| format!("{version} (tested {})", agent.tested_version),
    )?;
    let hooks = dir.join(".cursor");
    let written = fs::create_dir_all(&hooks)
        .and_then(|()| fs::write(hooks.join("hooks.json"), CURSOR_VERIFY_HOOKS))
        .map_err(|error| format!("could not write {}: {error}", hooks.display()));
    step(checks, "hooks", written, |()| {
        "wrote .cursor/hooks.json in the temporary directory".to_string()
    })?;
    let mut command = Command::new(agent.command);
    command.arg("acp").current_dir(dir);
    let spawned = spawn_piped(command)
        .map_err(|error| format!("could not start `{} acp`: {error}", agent.command));
    let Some((mut child, stdin, stdout, stderr)) = step(checks, "spawn", spawned, |_| {
        format!("`{} acp` started", agent.command)
    }) else {
        return version;
    };
    drain_stderr(stderr, CappedLog::new(None, "verify"));
    let deadline = if live { LIVE_DEADLINE } else { FREE_DEADLINE };
    let done = watchdog(child.id(), deadline);
    let (update_tx, updates) = queue::unbounded();
    let probe = Probe {
        allowing: Arc::new(AtomicBool::new(false)),
        permissions: Arc::default(),
        replies: Arc::default(),
        updates,
    };
    let (allowing, seen, replies) = (
        Arc::clone(&probe.allowing),
        Arc::clone(&probe.permissions),
        Arc::clone(&probe.replies),
    );
    let outcome = block_on(
        Client
            .builder()
            .on_receive_request(
                async move |request: RequestPermissionRequest, responder, _cx| {
                    let reply = permission_reply(&request, allowing.load(Ordering::SeqCst), &seen);
                    responder.respond(reply).ok();
                    Ok(())
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_notification(
                async move |notification: SessionNotification, _cx| {
                    let update = serde_json::to_value(&notification.update).unwrap_or_default();
                    if update["sessionUpdate"] == "agent_message_chunk" {
                        replies.fetch_add(1, Ordering::SeqCst);
                    }
                    update_tx.unbounded_send(()).ok();
                    Ok(())
                },
                agent_client_protocol::on_receive_notification!(),
            )
            .connect_with(
                ByteStreams::new(
                    blocking::Unblock::new(stdin),
                    blocking::Unblock::new(stdout),
                ),
                {
                    let (dir, checks) = (dir.to_path_buf(), checks.clone());
                    async move |connection: ConnectionTo<Agent>| {
                        run_checks(connection, dir, live, checks, probe).await
                    }
                },
            ),
    );
    done.send(()).ok();
    stop_child(&mut child);
    if let Err(error) = outcome {
        checks.add(CheckResult::fail(
            "connection",
            format!("{} agent connection failed: {error}", agent.name),
        ));
    }
    version
}
