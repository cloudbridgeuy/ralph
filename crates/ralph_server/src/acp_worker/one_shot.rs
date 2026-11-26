use super::live::{decide, Decision, PermissionPolicy};
use super::permissions::{respond_cancelled, respond_selected, RawPermissionRequest};
use super::*;
use agent_client_protocol::schema::v1::PromptResponse;
use agent_client_protocol::JsonRpcNotification;
use ralph_core::stream::{ContentBlock, StreamEvent};
use ralph_core::worker::{agy_stream_events, AgyEvent, AgyUsage};
use serde_json::Value;

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Collected {
    text: String,
    tool_calls: Vec<String>,
}

pub(super) fn fold(collected: &mut Collected, update: &Value) {
    for event in agy_stream_events(&AgyEvent::Update(update)) {
        let StreamEvent::Assistant(assistant) = event else {
            continue;
        };
        for block in assistant.message.content {
            match block {
                ContentBlock::Text { text } => collected.text.push_str(&text),
                ContentBlock::ToolUse { name, .. } => collected.tool_calls.push(name),
            }
        }
    }
}

pub struct AcpOneShot {
    pub text: String,
    pub tool_calls: Vec<String>,
    pub usage: Option<AgyUsage>,
    pub stop_reason: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OneShotError {
    TimedOut(Duration),
    Failed(String),
}

impl std::fmt::Display for OneShotError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OneShotError::TimedOut(timeout) => {
                write!(formatter, "agy timed out after {}s", timeout.as_secs())
            }
            OneShotError::Failed(message) => formatter.write_str(message),
        }
    }
}

fn reply_error(error: mpsc::RecvTimeoutError, timeout: Duration) -> OneShotError {
    match error {
        mpsc::RecvTimeoutError::Timeout => OneShotError::TimedOut(timeout),
        mpsc::RecvTimeoutError::Disconnected => {
            OneShotError::Failed("agy one-shot ended without a reply".to_string())
        }
    }
}

pub struct OneShotRequest<'a> {
    pub cwd: &'a Path,
    pub prompt: &'a str,
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub timeout: Duration,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, JsonRpcNotification)]
#[serde(transparent)]
#[notification(method = "session/update")]
struct RawUpdate(Value);

fn answer(request: &RawPermissionRequest, responder: Responder<RequestPermissionResponse>) {
    match decide(&PermissionPolicy::RejectAll, &request.0) {
        Some(Decision::Select(option_id)) => respond_selected(responder, option_id),
        Some(Decision::Ask(_)) | None => respond_cancelled(responder),
    }
}

async fn converse(
    connection: &ConnectionTo<Agent>,
    opening: &Opening,
    prompt: &str,
    collected: &Mutex<Collected>,
) -> Result<AcpOneShot, String> {
    initialize(connection, &opening.agent)
        .await
        .map_err(|error| format!("agy initialize failed: {}", error.message))??;
    let (session, selection) = agy_settings::open(connection, opening).await?;
    let response: Result<PromptResponse, String> = connection
        .send_request(PromptRequest::new(
            session.clone(),
            vec![prompt.to_string().into()],
        ))
        .block_task()
        .await
        .map_err(|error| format!("agy prompt failed: {}", error.message));
    connection
        .send_request(CloseSessionRequest::new(session))
        .block_task()
        .await
        .ok();
    let response = response?;
    let Collected { text, tool_calls } = std::mem::take(&mut *locked(collected));
    Ok(AcpOneShot {
        text,
        tool_calls,
        usage: response.usage.as_ref().map(agy_usage::raw_usage),
        stop_reason: serde_json::to_value(response.stop_reason)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_default(),
        model: selection.model,
    })
}

pub fn run(request: &OneShotRequest) -> Result<AcpOneShot, OneShotError> {
    let mut command = agy_settings::command(AGY_AGENT.command, PermissionMode::Ask);
    command.current_dir(request.cwd);
    let (mut child, stdin, stdout, stderr) = spawn_in_shell(command).map_err(|error| {
        OneShotError::Failed(format!(
            "could not start `{} acp`: {error}",
            AGY_AGENT.command
        ))
    })?;
    let log = CappedLog::new(None, "one-shot");
    drain_stderr(stderr, Arc::clone(&log));
    let (sender, receiver) = mpsc::channel();
    let opening = Opening {
        agent: AGY_AGENT,
        launch: Launch::Fresh,
        cwd: request.cwd.to_path_buf(),
        servers: Vec::new(),
        worker_id: "one-shot".to_string(),
        model: request.model.map(str::to_string),
        permission_mode: PermissionMode::Ask,
        effort: request.effort.map(str::to_string),
        updates: Arc::new(Mutex::new(None)),
        inbox: PathBuf::new(),
        scheduled: false,
    };
    let prompt = request.prompt.to_string();
    thread::spawn(move || {
        let collected = Arc::new(Mutex::new(Collected::default()));
        let on_update = Arc::clone(&opening.updates);
        let on_collect = Arc::clone(&collected);
        let failed = sender.clone();
        let outcome = block_on(
            Client
                .builder()
                .on_receive_request(
                    async move |request: RawPermissionRequest, responder, _cx| {
                        answer(&request, responder);
                        Ok(())
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_notification(
                    async move |notification: RawUpdate, _cx| {
                        let session = notification.0["sessionId"].as_str().unwrap_or_default();
                        let update = &notification.0["update"];
                        agy_settings::observe(&on_update, session, update);
                        fold(&mut locked(&on_collect), update);
                        Ok(())
                    },
                    agent_client_protocol::on_receive_notification!(),
                )
                .connect_with(
                    transport::streams(stdin, stdout, log),
                    |connection: ConnectionTo<Agent>| async move {
                        let result = converse(&connection, &opening, &prompt, &collected).await;
                        sender.send(result).ok();
                        Ok(())
                    },
                ),
        );
        if let Err(error) = outcome {
            failed
                .send(Err(format!("agy connection failed: {error}")))
                .ok();
        }
    });
    let result = match receiver.recv_timeout(request.timeout) {
        Ok(result) => result.map_err(OneShotError::Failed),
        Err(error) => Err(reply_error(error, request.timeout)),
    };
    stop_child(&mut child);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_receive_timeout_is_a_timed_out_error() {
        let timeout = Duration::from_secs(7);
        let error = reply_error(mpsc::RecvTimeoutError::Timeout, timeout);
        assert_eq!(error, OneShotError::TimedOut(timeout));
        assert_eq!(error.to_string(), "agy timed out after 7s");
    }

    #[test]
    fn a_dropped_sender_is_a_failure_not_a_timeout() {
        let error = reply_error(mpsc::RecvTimeoutError::Disconnected, Duration::from_secs(7));
        assert_eq!(
            error,
            OneShotError::Failed("agy one-shot ended without a reply".to_string())
        );
    }

    #[test]
    fn fold_joins_message_chunks_and_skips_thoughts() {
        let mut collected = Collected::default();
        for update in [
            json!({"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "hmm"}}),
            json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Hello "}}),
            json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "world"}}),
            json!({"sessionUpdate": "usage_update", "used": 3, "size": 9}),
        ] {
            fold(&mut collected, &update);
        }
        assert_eq!(collected.text, "Hello world");
        assert!(collected.tool_calls.is_empty());
    }

    #[test]
    fn fold_records_tool_call_names() {
        let mut collected = Collected::default();
        fold(
            &mut collected,
            &json!({"sessionUpdate": "tool_call", "toolCallId": "c", "name": "run_command", "title": "ls"}),
        );
        fold(
            &mut collected,
            &json!({"sessionUpdate": "tool_call", "toolCallId": "d", "title": "Read file"}),
        );
        fold(
            &mut collected,
            &json!({"sessionUpdate": "tool_call_update", "toolCallId": "c", "status": "completed"}),
        );
        assert_eq!(collected.tool_calls, ["run_command", "Read file"]);
    }
}
