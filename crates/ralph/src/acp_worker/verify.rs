use super::*;
use crate::subprocess::kill_process_group;
use crate::worker_verify::{step, Checks};
use ralph_core::worker::{
    cancel_check, cursor_verify_leftover, cursor_verify_project, permission_check, probed_version,
    prompt_check, BlockedContent, CheckResult, PermissionSetup, TurnSummary, ACP_PROTOCOL_VERSION,
    AUTH_REQUIRED_CODE, CURSOR_HOOKS_FILE, CURSOR_VERIFY_HOOKS, VERSION_PROBE_LIMIT,
};
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::sync::atomic::AtomicUsize;

const FREE_DEADLINE: Duration = Duration::from_secs(90);
const LIVE_DEADLINE: Duration = Duration::from_secs(420);
const KILL_GRACE: Duration = Duration::from_millis(200);
const VERSION_POLL: Duration = Duration::from_millis(25);
const REPLY_PROMPT: &str = "Reply with the single word: ok";
const PERMISSION_PROMPT: &str = "Run the shell command `date +%Y` and reply with its output.";
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
    live: Option<PermissionSetup>,
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
    if let Some(setup) = live {
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
            &setup,
        ));
        checks.add(cancel_check(
            &session.turn(CANCEL_PROMPT, Plan::Cancel).await,
            CANCELLED,
        ));
    }
    Ok(())
}

pub(super) fn agent_version_probe(agent: &AcpAgent) -> VersionProbe {
    let spawned = Command::new(agent.command)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => return VersionProbe::Failed(error.to_string()),
    };
    let Some(mut stdout) = child.stdout.take() else {
        kill_process_group(&mut child);
        return VersionProbe::Failed("no stdout".to_string());
    };
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut text = String::new();
        let _ = sender.send(stdout.read_to_string(&mut text).map(|_| text));
    });
    let deadline = Instant::now() + VERSION_PROBE_LIMIT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => thread::sleep(VERSION_POLL),
            Ok(None) => {
                kill_process_group(&mut child);
                return VersionProbe::TimedOut;
            }
            Err(error) => {
                kill_process_group(&mut child);
                return VersionProbe::Failed(error.to_string());
            }
        }
    }
    match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(Ok(text)) => VersionProbe::Exited(text),
        Ok(Err(error)) => VersionProbe::Failed(error.to_string()),
        Err(_) => VersionProbe::TimedOut,
    }
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
    let leftover = fs::canonicalize(dir)
        .ok()
        .and_then(|real| cursor_verify_leftover(&home, &real, &project))
        .filter(|leftover| !leftover.exists());
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
        exercise(live.then(|| PermissionSetup::cursor(&hooks)), dir, checks);
    }
    let removed = fs::remove_dir_all(&project)
        .map_err(|error| format!("could not remove {}: {error}", project.display()));
    step(checks, "hooks cleanup", removed, |()| {
        format!("removed {}", project.display())
    });
    if let Some(leftover) = leftover.filter(|leftover| leftover.exists()) {
        let removed = fs::remove_dir_all(&leftover)
            .map_err(|error| format!("could not remove {}: {error}", leftover.display()));
        step(checks, "cursor cleanup", removed, |()| {
            format!("removed {}", leftover.display())
        });
    }
    version
}

fn exercise(live: Option<PermissionSetup>, dir: &Path, checks: &Checks) {
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
    drain_stderr(stderr, CappedLog::new(None, "verify"));
    let deadline = if live.is_some() {
        LIVE_DEADLINE
    } else {
        FREE_DEADLINE
    };
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
}
