use super::permissions::{respond_cancelled, respond_selected, RawPermissionRequest};
use super::*;
use agent_client_protocol::schema::v1::PromptResponse;
use agent_client_protocol::JsonRpcNotification;
use ralph_core::chat_tools::{tool_verdict, ToolPolicy, ToolVerdict};
use ralph_core::worker::AgyUsage;
use ralph_core::worker::{
    claude_tool_name, refusal, Choice, ClaudePermission, PermissionOption, StopReason,
};
use serde_json::Value;

pub type AnswerSender = oneshot::Sender<Choice>;

#[allow(dead_code, reason = "the chat session constructs this")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionPolicy {
    RejectAll,
    Chat(ToolPolicy),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Select(String),
    Ask(ClaudePermission),
}

#[allow(dead_code, reason = "the chat session constructs this")]
pub enum AcpEvent {
    Update(Value),
    Ask(ClaudePermission, AnswerSender),
    Done {
        stop_reason: StopReason,
        usage: Option<AgyUsage>,
    },
    Failed(String),
}

pub struct AcpOpen<'a> {
    pub cwd: &'a Path,
    pub servers: Vec<McpServer>,
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub policy: PermissionPolicy,
}

#[allow(dead_code, reason = "the chat session constructs this")]
enum Command {
    Prompt(String),
    Cancel,
    Close,
}

pub struct AcpLive {
    child: Child,
    commands: queue::UnboundedSender<Command>,
    events: mpsc::Receiver<AcpEvent>,
    model: String,
}

fn option_of_kind<'a>(options: &'a [PermissionOption], kind: &str) -> Option<&'a PermissionOption> {
    options.iter().find(|option| option.kind == kind)
}

pub fn decide(policy: &PermissionPolicy, request: &Value) -> Option<Decision> {
    let content = permission_content(request);
    match policy {
        PermissionPolicy::RejectAll => refusal(&content)
            .option_id()
            .map(|option| Decision::Select(option.to_string())),
        PermissionPolicy::Chat(tools) => {
            let allowed =
                tool_verdict(tools, &claude_tool_name(&content.tool)) == ToolVerdict::Allow;
            let allow_once = option_of_kind(&content.options, "allow_once").filter(|_| allowed);
            Some(match allow_once {
                Some(option) => Decision::Select(option.option_id.clone()),
                None => Decision::Ask(ClaudePermission {
                    input: request["toolCall"]["rawInput"].clone(),
                    content,
                }),
            })
        }
    }
}

fn option_for(choice: Choice, options: &[PermissionOption]) -> Option<String> {
    let kinds: &[&str] = match choice {
        Choice::AllowOnce => &["allow_once"],
        Choice::AllowAlways => &["allow_always", "allow_once"],
        Choice::Deny => &["reject_once"],
    };
    kinds
        .iter()
        .find_map(|kind| option_of_kind(options, kind))
        .map(|option| option.option_id.clone())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, JsonRpcNotification)]
#[serde(transparent)]
#[notification(method = "session/update")]
struct RawUpdate(Value);

fn answer(
    policy: &PermissionPolicy,
    request: &RawPermissionRequest,
    responder: Responder<RequestPermissionResponse>,
    events: &mpsc::Sender<AcpEvent>,
    connection: &ConnectionTo<Agent>,
) {
    match decide(policy, &request.0) {
        Some(Decision::Select(option_id)) => respond_selected(responder, option_id),
        Some(Decision::Ask(ask)) => {
            let options = ask.content.options.clone();
            let (sender, receiver) = oneshot::channel();
            events.send(AcpEvent::Ask(ask, sender)).ok();
            let waiting = async move {
                match receiver
                    .await
                    .ok()
                    .and_then(|choice| option_for(choice, &options))
                {
                    Some(option_id) => respond_selected(responder, option_id),
                    None => respond_cancelled(responder),
                }
                Ok(())
            };
            if connection.spawn(waiting).is_err() {
                events
                    .send(AcpEvent::Failed("agy connection is closed".to_string()))
                    .ok();
            }
        }
        None => respond_cancelled(responder),
    }
}

async fn prompt(
    connection: &ConnectionTo<Agent>,
    session: &SessionId,
    text: String,
    commands: &mut queue::UnboundedReceiver<Command>,
) -> Option<AcpEvent> {
    let request = connection
        .send_request(PromptRequest::new(session.clone(), vec![text.into()]))
        .block_task()
        .fuse();
    futures::pin_mut!(request);
    loop {
        select! {
            response = request => return Some(finished(response)),
            command = commands.next() => match command {
                Some(Command::Cancel) => {
                    connection.send_notification(CancelNotification::new(session.clone())).ok();
                }
                Some(Command::Prompt(_)) => {}
                Some(Command::Close) | None => return None,
            },
        }
    }
}

fn finished(response: Result<PromptResponse, agent_client_protocol::Error>) -> AcpEvent {
    match response {
        Ok(response) => AcpEvent::Done {
            stop_reason: StopReason::parse(
                serde_json::to_value(response.stop_reason)
                    .ok()
                    .as_ref()
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            ),
            usage: response.usage.as_ref().map(agy_usage::raw_usage),
        },
        Err(error) => AcpEvent::Failed(format!("agy prompt failed: {}", error.message)),
    }
}

async fn converse(
    connection: &ConnectionTo<Agent>,
    opening: &Opening,
    ready: mpsc::Sender<Result<String, String>>,
    mut commands: queue::UnboundedReceiver<Command>,
    events: &mpsc::Sender<AcpEvent>,
) -> Result<(), agent_client_protocol::Error> {
    let opened = match initialize(connection, &opening.agent).await? {
        Ok(_) => agy_settings::open(connection, opening).await,
        Err(message) => Err(message),
    };
    let (session, selection) = match opened {
        Ok(opened) => opened,
        Err(message) => {
            ready.send(Err(message)).ok();
            return Ok(());
        }
    };
    ready.send(Ok(selection.model)).ok();
    while let Some(command) = commands.next().await {
        match command {
            Command::Prompt(text) => {
                match prompt(connection, &session, text, &mut commands).await {
                    Some(event) => events.send(event).ok(),
                    None => break,
                };
            }
            Command::Cancel => {
                connection
                    .send_notification(CancelNotification::new(session.clone()))
                    .ok();
            }
            Command::Close => break,
        }
    }
    connection
        .send_request(CloseSessionRequest::new(session))
        .block_task()
        .await
        .ok();
    Ok(())
}

impl AcpLive {
    pub fn open(open: AcpOpen) -> Result<Self, String> {
        let mut command = agy_settings::command(AGY_AGENT.command, PermissionMode::Ask);
        command.current_dir(open.cwd);
        let (mut child, stdin, stdout, stderr) = spawn_in_shell(command)
            .map_err(|error| format!("could not start `{} acp`: {error}", AGY_AGENT.command))?;
        let log = CappedLog::new(None, "live");
        drain_stderr(stderr, Arc::clone(&log));
        let (events, receiver) = mpsc::channel();
        let (commands, command_rx) = queue::unbounded();
        let (ready, ready_rx) = mpsc::channel();
        let opening = Opening {
            agent: AGY_AGENT,
            launch: Launch::Fresh,
            cwd: open.cwd.to_path_buf(),
            servers: open.servers,
            worker_id: "live".to_string(),
            model: open.model.map(str::to_string),
            permission_mode: PermissionMode::Ask,
            effort: open.effort.map(str::to_string),
            updates: Arc::new(Mutex::new(None)),
            inbox: PathBuf::new(),
            scheduled: false,
        };
        let policy = open.policy;
        thread::spawn(move || {
            let on_update = Arc::clone(&opening.updates);
            let update_events = events.clone();
            let request_events = events.clone();
            let converse_events = events.clone();
            let outcome = block_on(
                Client
                    .builder()
                    .on_receive_request(
                        async move |request: RawPermissionRequest, responder, cx| {
                            answer(&policy, &request, responder, &request_events, &cx);
                            Ok(())
                        },
                        agent_client_protocol::on_receive_request!(),
                    )
                    .on_receive_notification(
                        async move |notification: RawUpdate, _cx| {
                            let session = notification.0["sessionId"].as_str().unwrap_or_default();
                            let update = &notification.0["update"];
                            agy_settings::observe(&on_update, session, update);
                            update_events.send(AcpEvent::Update(update.clone())).ok();
                            Ok(())
                        },
                        agent_client_protocol::on_receive_notification!(),
                    )
                    .connect_with(
                        transport::streams(stdin, stdout, log),
                        |connection: ConnectionTo<Agent>| async move {
                            converse(&connection, &opening, ready, command_rx, &converse_events)
                                .await
                        },
                    ),
            );
            if let Err(error) = outcome {
                events
                    .send(AcpEvent::Failed(format!("agy connection failed: {error}")))
                    .ok();
            }
        });
        match ready_rx.recv_timeout(READY_TIMEOUT) {
            Ok(Ok(model)) => Ok(Self {
                child,
                commands,
                events: receiver,
                model,
            }),
            Ok(Err(message)) => {
                stop_child(&mut child);
                Err(message)
            }
            Err(_) => {
                stop_child(&mut child);
                Err(format!(
                    "agy did not become ready within {}s",
                    READY_TIMEOUT.as_secs()
                ))
            }
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn prompt(&self, text: &str) -> Result<(), String> {
        self.commands
            .unbounded_send(Command::Prompt(text.to_string()))
            .map_err(|_| "agy session is closed".to_string())
    }

    pub fn next(&self, wait: Duration) -> Option<AcpEvent> {
        self.events.recv_timeout(wait).ok()
    }

    #[allow(dead_code, reason = "the chat session constructs this")]
    pub fn cancel(&self) {
        self.commands.unbounded_send(Command::Cancel).ok();
    }

    #[allow(dead_code, reason = "the chat session constructs this")]
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub fn close(self) {
        self.commands.unbounded_send(Command::Close).ok();
        while self.events.recv_timeout(SHUTDOWN_GRACE).is_ok() {}
    }
}

impl Drop for AcpLive {
    fn drop(&mut self) {
        stop_child(&mut self.child);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::chat_tools::chat_tool_policy;
    use serde_json::{json, Value};

    fn request(name: &str, kinds: &[&str]) -> Value {
        let options: Vec<Value> = kinds
            .iter()
            .enumerate()
            .map(|(index, kind)| json!({"optionId": format!("o{index}"), "name": *kind, "kind": *kind}))
            .collect();
        json!({"sessionId": "s", "toolCall": {"toolCallId": "c", "name": name, "title": "do it", "rawInput": {"command": "ls"}}, "options": options})
    }

    fn chat() -> PermissionPolicy {
        PermissionPolicy::Chat(chat_tool_policy(["ralph-chat"]))
    }

    #[test]
    fn decide_reject_all_selects_reject_option() {
        let chosen = decide(
            &PermissionPolicy::RejectAll,
            &request("run_command", &["allow_once", "reject_once"]),
        );
        assert_eq!(chosen, Some(Decision::Select("o1".to_string())));
        let none = decide(&PermissionPolicy::RejectAll, &request("x", &["allow_once"]));
        assert_eq!(none, None);
    }

    #[test]
    fn decide_chat_allows_chat_mcp_tool() {
        let chosen = decide(
            &chat(),
            &request("mcp_ralph-chat_report", &["reject_once", "allow_once"]),
        );
        assert_eq!(chosen, Some(Decision::Select("o1".to_string())));
    }

    #[test]
    fn decide_chat_asks_for_run_command() {
        let Some(Decision::Ask(ask)) = decide(
            &chat(),
            &request("run_command", &["allow_once", "reject_once"]),
        ) else {
            panic!("expected Ask")
        };
        assert_eq!(ask.content.tool, "run_command");
        assert_eq!(ask.content.summary, "do it");
        assert_eq!(ask.input, json!({"command": "ls"}));
    }

    #[test]
    fn decide_chat_asks_when_no_allow_option() {
        let chosen = decide(&chat(), &request("mcp_ralph-chat_report", &["reject_once"]));
        assert!(matches!(chosen, Some(Decision::Ask(_))));
    }
}
