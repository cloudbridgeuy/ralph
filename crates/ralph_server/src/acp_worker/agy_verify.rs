use super::*;
use crate::worker_verify::{step, Checks};
use agent_client_protocol::schema::v1::{CloseSessionRequest, McpServerStdio};
use ralph_core::worker::{CheckResult, AGY_AGENT};

mod evidence;
mod live;

const STUB: &str = r#"import json, sys
from pathlib import Path
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request:
        continue
    method = request.get('method')
    reply = {'jsonrpc': '2.0', 'id': request['id']}
    if method == 'initialize':
        reply['result'] = {'protocolVersion': request['params']['protocolVersion'], 'capabilities': {'tools': {}}, 'serverInfo': {'name': 'ralph-probe', 'version': '1'}}
    elif method == 'tools/list':
        Path(sys.argv[1]).write_text('listed')
        reply['result'] = {'tools': [{'name': 'ralph_probe', 'description': 'Discovery probe', 'inputSchema': {'type': 'object', 'properties': {}}}]}
    elif method == 'ping':
        reply['result'] = {}
    else:
        reply['error'] = {'code': -32601, 'message': 'Method not found'}
    print(json.dumps(reply), flush=True)
"#;

fn server(dir: &Path, receipt: &str) -> McpServer {
    McpServer::Stdio(McpServerStdio::new("ralph-probe", "python3").args(vec![
        "-I".into(),
        dir.join("mcp-probe.py").to_string_lossy().into_owned(),
        dir.join(receipt).to_string_lossy().into_owned(),
    ]))
}

async fn discovery(path: PathBuf) -> Result<String, String> {
    blocking::unblock(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if matches!(fs::read_to_string(&path).as_deref(), Ok("listed")) {
                return Ok("MCP tools/list received".to_string());
            }
            thread::sleep(Duration::from_millis(50));
        }
        Err(
            "agy did not discover the MCP tools; ensure python3 is installed and available on PATH"
                .to_string(),
        )
    })
    .await
}

async fn close(connection: &ConnectionTo<Agent>, id: &SessionId) -> Result<String, String> {
    connection
        .send_request(CloseSessionRequest::new(id.clone()))
        .block_task()
        .await
        .map(|_| "session closed".to_string())
        .map_err(|error| agy_settings::session_failure(i32::from(error.code), &error.message))
}

fn identity(init: &InitializeResponse, version: &str) -> Result<(), String> {
    if let Some(info) = &init.agent_info {
        if info.name != "Antigravity ACP Server" || info.version != version {
            return Err(format!(
                "agy ACP identity {} {} does not match agy {version}",
                info.name, info.version
            ));
        }
    }
    Ok(())
}

fn opening(dir: &Path, receipt: &str, updates: agy_settings::Updates) -> Opening {
    Opening {
        agent: AGY_AGENT,
        launch: Launch::Fresh,
        cwd: dir.to_path_buf(),
        server: server(dir, receipt),
        worker_id: "verify".to_string(),
        model: Some("gemini-3.8-flash-low".to_string()),
        permission_mode: PermissionMode::Ask,
        effort: Some("low".to_string()),
        updates,
        inbox: dir.join("inbox"),
    }
}

fn settings(selection: &agy_settings::Selection) -> Result<String, String> {
    if selection.model == "gemini-3.8-flash-low" && selection.effort.as_deref() == Some("low") {
        Ok("Flash model and low effort confirmed".to_string())
    } else {
        Err(format!(
            "agy returned unexpected settings: {} {:?}",
            selection.model, selection.effort
        ))
    }
}

struct Run {
    dir: PathBuf,
    checks: Checks,
    version: String,
    updates: agy_settings::Updates,
    live: bool,
    allow_report: Arc<AtomicBool>,
}

async fn run(
    connection: ConnectionTo<Agent>,
    run: Run,
) -> Result<(), agent_client_protocol::Error> {
    let Run {
        dir,
        checks,
        version,
        updates,
        live,
        allow_report,
    } = run;
    let init = match initialize(&connection, &AGY_AGENT).await {
        Ok(result) => result.and_then(|init| identity(&init, &version).map(|()| init)),
        Err(error) => Err(format!("agy initialize failed: {}", error.message)),
    };
    let Some(init) = step(&checks, "initialize", init, |_| {
        "ACP protocol 1".to_string()
    }) else {
        return Ok(());
    };
    if !init.agent_capabilities.load_session {
        checks.add(CheckResult::fail(
            "session/load",
            "agy does not advertise session loading",
        ));
        return Ok(());
    }
    let new = opening(&dir, "new.receipt", Arc::clone(&updates));
    let created = agy_settings::open(&connection, &new).await;
    let Some((id, selected)) = step(&checks, "session/new", created, |(id, _)| {
        format!("session {id}")
    }) else {
        return Ok(());
    };
    if step(&checks, "settings/new", settings(&selected), Clone::clone).is_none() {
        close(&connection, &id).await.ok();
        return Ok(());
    }
    let found = discovery(dir.join("new.receipt")).await;
    if step(&checks, "mcp/new", found, Clone::clone).is_none() {
        close(&connection, &id).await.ok();
        return Ok(());
    }
    let closed = close(&connection, &id).await;
    if step(&checks, "session/close", closed, Clone::clone).is_none() {
        return Ok(());
    }
    let load = opening(&dir, "load.receipt", Arc::clone(&updates));
    let loaded = agy_settings::load(&connection, &load, id.clone()).await;
    let Some((_, selected)) = step(&checks, "session/load", loaded, |_| {
        "session loaded".to_string()
    }) else {
        return Ok(());
    };
    if step(&checks, "settings/load", settings(&selected), Clone::clone).is_none() {
        close(&connection, &id).await.ok();
        return Ok(());
    }
    let found = discovery(dir.join("load.receipt")).await;
    if step(&checks, "mcp/load", found, Clone::clone).is_none() {
        close(&connection, &id).await.ok();
        return Ok(());
    }
    let closed = close(&connection, &id).await;
    if step(&checks, "session/close reload", closed, Clone::clone).is_some() && live {
        live::run(&connection, &dir, &checks, updates, &allow_report).await;
    }
    Ok(())
}

pub(crate) fn verify(live: bool, dir: &Path, checks: &Checks, version: &str, remaining: Duration) {
    if remaining.is_zero() {
        checks.add(CheckResult::fail(
            "timeout",
            "agy verify budget expired during version probing",
        ));
        return;
    }
    let started = Instant::now();
    let canonical = fs::canonicalize(dir)
        .map_err(|error| format!("could not resolve verify directory: {error}"));
    let Some(dir) = step(checks, "directory", canonical, |dir| {
        dir.display().to_string()
    }) else {
        return;
    };
    let dir = dir.as_path();
    let written = fs::write(dir.join("mcp-probe.py"), STUB)
        .map_err(|error| format!("could not write MCP probe: {error}"));
    if step(checks, "mcp probe", written, |_| {
        "MCP probe prepared".to_string()
    })
    .is_none()
    {
        return;
    }
    let evidence = if live {
        match evidence::Evidence::create() {
            Ok((evidence, path)) => {
                println!("agy permission evidence: {}", path.display());
                Some(evidence)
            }
            Err(error) => {
                checks.add(CheckResult::fail(
                    "evidence",
                    format!("could not create permission evidence: {error}"),
                ));
                return;
            }
        }
    } else {
        None
    };
    let mut command = Command::new(AGY_AGENT.command);
    command.arg("acp").current_dir(dir);
    let spawned =
        spawn_piped(command).map_err(|error| format!("could not start `agy acp`: {error}"));
    let Some((mut child, stdin, stdout, stderr)) =
        step(checks, "spawn", spawned, |_| "agy acp started".to_string())
    else {
        return;
    };
    let log = CappedLog::new(None, "verify");
    drain_stderr(stderr, Arc::clone(&log));
    let (done, waiting) = mpsc::channel();
    let timed_out = Arc::new(AtomicBool::new(false));
    let timeout_flag = Arc::clone(&timed_out);
    let group = child.id();
    let remaining = remaining.saturating_sub(started.elapsed());
    let watchdog = thread::spawn(move || {
        if let Err(mpsc::RecvTimeoutError::Timeout) = waiting.recv_timeout(remaining) {
            timeout_flag.store(true, Ordering::SeqCst);
            stop_groups(&[group], Duration::from_millis(200), || false);
        }
    });
    let allow_report = Arc::new(AtomicBool::new(false));
    let permission = Arc::clone(&allow_report);
    let updates = Arc::new(Mutex::new(None));
    let observer = Arc::clone(&updates);
    let version = version.to_string();
    let outcome = block_on(
        Client
            .builder()
            .on_receive_request(
                async move |request: RawPermissionRequest, responder, _cx| {
                    let content = permission_content(&request.0);
                    let option = agy_auto_allow_option(&request.0, &content)
                        .filter(|_| permission.load(Ordering::SeqCst));
                    let outcome = match option {
                        Some(option) => RequestPermissionOutcome::Selected(
                            SelectedPermissionOutcome::new(option.option_id.clone()),
                        ),
                        None => RequestPermissionOutcome::Cancelled,
                    };
                    responder
                        .respond(RequestPermissionResponse::new(outcome))
                        .ok();
                    Ok(())
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_notification(
                async move |notification: SessionNotification, _cx| {
                    if let Ok(update) = serde_json::to_value(&notification.update) {
                        agy_settings::observe(
                            &observer,
                            &notification.session_id.to_string(),
                            &update,
                        );
                    }
                    Ok(())
                },
                agent_client_protocol::on_receive_notification!(),
            )
            .connect_with(
                agent_client_protocol::ByteStreams::new(
                    blocking::Unblock::new(evidence::Capture::new(
                        stdin,
                        evidence.clone(),
                        "client_to_agent",
                    )),
                    blocking::Unblock::new(evidence::Capture::new(
                        stdout,
                        evidence.clone(),
                        "agent_to_client",
                    )),
                ),
                {
                    let (dir, checks) = (dir.to_path_buf(), checks.clone());
                    async move |connection| {
                        run(
                            connection,
                            Run {
                                dir,
                                checks,
                                version,
                                updates,
                                live,
                                allow_report,
                            },
                        )
                        .await
                    }
                },
            ),
    );
    done.send(()).ok();
    watchdog.join().ok();
    let exited = exited_unreaped(&child);
    stop_groups(&[group], Duration::from_millis(200), || {
        !exited_unreaped(&child)
    });
    let status = child.wait();
    let failed = checks
        .results()
        .iter()
        .any(|check| check.blocking && check.verdict != ralph_core::worker::Verdict::Pass);
    if let Some(error) = evidence.and_then(|evidence| evidence.error()) {
        if failed {
            eprintln!("could not record permission evidence: {error}");
        } else {
            checks.add(CheckResult::fail(
                "evidence",
                format!("could not record permission evidence: {error}"),
            ));
        }
        return;
    }
    if failed {
        return;
    }
    if timed_out.load(Ordering::SeqCst) {
        checks.add(CheckResult::fail(
            "timeout",
            format!(
                "agy verification exceeded {} seconds",
                ralph_core::worker::verify_limit(live).as_secs()
            ),
        ));
    } else if let Err(ref error) = outcome {
        checks.add(CheckResult::fail(
            "connection",
            format!("agy ACP connection failed: {error}"),
        ));
    } else if exited && !matches!(status.as_ref(), Ok(status) if status.success()) {
        checks.add(CheckResult::fail(
            "connection",
            format!("agy acp exited during verification: {status:?}"),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_binding_has_a_separate_receipt_and_the_same_server(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let dir = Path::new("/tmp/verify");
        let first = serde_json::to_value(server(dir, "new.receipt"))?;
        let second = serde_json::to_value(server(dir, "load.receipt"))?;
        assert_eq!(first["name"], second["name"]);
        assert_eq!(first["command"], second["command"]);
        assert_eq!(first["args"][0], second["args"][0]);
        assert_eq!(first["args"][1], second["args"][1]);
        assert_ne!(first["args"][2], second["args"][2]);
        Ok(())
    }

    #[test]
    fn identity_and_settings_must_match_the_probe() -> Result<(), Box<dyn std::error::Error>> {
        let make = |name, version| {
            serde_json::from_value::<InitializeResponse>(serde_json::json!({
                "protocolVersion": 1, "agentInfo": {"name": name, "version": version}
            }))
        };
        assert!(identity(&make("Antigravity ACP Server", "1.5.12")?, "1.5.12").is_ok());
        assert!(identity(&make("Antigravity ACP Server", "1.5.13")?, "1.5.12").is_err());
        assert!(identity(&make("other", "1.5.12")?, "1.5.12").is_err());
        assert!(settings(&agy_settings::Selection {
            model: "gemini-3.8-flash-low".into(),
            effort: Some("low".into())
        })
        .is_ok());
        assert!(settings(&agy_settings::Selection {
            model: "gemini-3.8-flash-low".into(),
            effort: Some("high".into())
        })
        .is_err());
        Ok(())
    }

    #[test]
    fn authentication_failure_gives_sign_in_guidance() {
        assert!(
            agy_settings::session_failure(ralph_core::worker::AUTH_REQUIRED_CODE, "denied")
                .contains("sign in")
        );
        assert!(agy_settings::session_failure(-32603, "broken").contains("broken"));
    }
}
