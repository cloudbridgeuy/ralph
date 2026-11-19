use super::*;
use crate::worker_verify::{step, Checks};
use agent_client_protocol::schema::v1::{CloseSessionRequest, McpServerStdio};
use ralph_core::worker::{CheckResult, AGY_AGENT, AUTH_REQUIRED_CODE};

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

fn session_failure(code: i32, message: &str) -> String {
    if code == AUTH_REQUIRED_CODE {
        "agy is not authenticated; sign in with `agy` and retry".into()
    } else {
        format!("agy session request failed: {message}")
    }
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
        .map_err(|error| session_failure(i32::from(error.code), &error.message))
}

async fn run(
    connection: ConnectionTo<Agent>,
    dir: PathBuf,
    checks: Checks,
) -> Result<(), agent_client_protocol::Error> {
    let init = match initialize(&connection, &AGY_AGENT).await {
        Ok(result) => result,
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
    let created = connection
        .send_request(
            NewSessionRequest::new(dir.clone()).mcp_servers(vec![server(&dir, "new.receipt")]),
        )
        .block_task()
        .await
        .map_err(|error| session_failure(i32::from(error.code), &error.message));
    let Some(created) = step(&checks, "session/new", created, |created| {
        format!("session {}", created.session_id)
    }) else {
        return Ok(());
    };
    let id = created.session_id;
    let found = discovery(dir.join("new.receipt")).await;
    if step(&checks, "mcp/new", found, Clone::clone).is_none() {
        close(&connection, &id).await.ok();
        return Ok(());
    }
    let closed = close(&connection, &id).await;
    if step(&checks, "session/close", closed, Clone::clone).is_none() {
        return Ok(());
    }
    let loaded = connection
        .send_request(
            LoadSessionRequest::new(id.clone(), dir.clone())
                .mcp_servers(vec![server(&dir, "load.receipt")]),
        )
        .block_task()
        .await
        .map_err(|error| session_failure(i32::from(error.code), &error.message));
    if step(&checks, "session/load", loaded, |_| {
        "session loaded".to_string()
    })
    .is_none()
    {
        return Ok(());
    }
    let found = discovery(dir.join("load.receipt")).await;
    if step(&checks, "mcp/load", found, Clone::clone).is_none() {
        close(&connection, &id).await.ok();
        return Ok(());
    }
    let closed = close(&connection, &id).await;
    step(&checks, "session/close reload", closed, Clone::clone);
    Ok(())
}

pub(crate) fn verify(live: bool, dir: &Path, checks: &Checks) {
    if live {
        checks.add(CheckResult::fail(
            "live",
            "agy live verification is not available yet",
        ));
        return;
    }
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
    let watchdog = thread::spawn(move || {
        if let Err(mpsc::RecvTimeoutError::Timeout) = waiting.recv_timeout(Duration::from_secs(90))
        {
            timeout_flag.store(true, Ordering::SeqCst);
            stop_groups(&[group], Duration::from_millis(200), || false);
        }
    });
    let outcome = block_on(Client.builder().connect_with(
        transport::streams(stdin, stdout, log),
        {
            let (dir, checks) = (dir.to_path_buf(), checks.clone());
            async move |connection| run(connection, dir, checks).await
        },
    ));
    done.send(()).ok();
    watchdog.join().ok();
    let exited = child.try_wait().ok().flatten();
    stop_child(&mut child);
    if timed_out.load(Ordering::SeqCst) {
        checks.add(CheckResult::fail(
            "timeout",
            "agy verification exceeded 90 seconds",
        ));
    } else if let Some(status) = exited.filter(|status| !status.success()) {
        checks.add(CheckResult::fail(
            "connection",
            format!("agy acp exited during verification: {status}"),
        ));
    } else if let Err(error) = outcome {
        checks.add(CheckResult::fail(
            "connection",
            format!("agy ACP connection failed: {error}"),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_binding_has_a_separate_receipt_and_the_same_server() {
        let dir = Path::new("/tmp/verify");
        let first = serde_json::to_value(server(dir, "new.receipt")).unwrap();
        let second = serde_json::to_value(server(dir, "load.receipt")).unwrap();
        assert_eq!(first["name"], second["name"]);
        assert_eq!(first["command"], second["command"]);
        assert_eq!(first["args"][0], second["args"][0]);
        assert_eq!(first["args"][1], second["args"][1]);
        assert_ne!(first["args"][2], second["args"][2]);
    }

    #[test]
    fn authentication_failure_gives_sign_in_guidance() {
        assert!(session_failure(AUTH_REQUIRED_CODE, "denied").contains("sign in"));
        assert!(session_failure(-32603, "broken").contains("broken"));
    }
}
