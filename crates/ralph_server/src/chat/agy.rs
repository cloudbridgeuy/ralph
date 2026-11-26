use crate::acp_worker::live::{AcpEvent, AcpLive, AcpOpen, AnswerSender, PermissionPolicy};
use crate::chat::backend::{ChatBackend, ChatLaunch, ChatSession, TurnIo};
use crate::subprocess::{
    Display, OutputGate, Pump, SpinnerSubprocessConfig, StreamingSubprocessResult, SubprocessError,
    EXIT_CODE_INTERRUPTED,
};
use agent_client_protocol::schema::v1::McpServer;
use ralph_core::cancel::CancelToken;
use ralph_core::chat_backend::{Caps, AGY};
use ralph_core::chat_memory::AgyNapError;
use ralph_core::chat_tools::ToolPolicy;
use ralph_core::sanitize::strip_controls;
use ralph_core::stream::blocks::{extract_response_text, VerboseToolsConfig};
use ralph_core::worker::{agy_notice, agy_stream_events, AgyEvent, Choice, Classes};
use serde_json::{json, Value};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(100);
const CANCEL_GRACE: Duration = Duration::from_secs(3);

pub struct AgySlot {
    pub cwd: PathBuf,
    pub model: String,
    pub effort: Option<String>,
    pub servers: Vec<McpServer>,
    pub policy: ToolPolicy,
}

pub struct AgySession {
    live: Option<AcpLive>,
    opening: Option<String>,
    config: SpinnerSubprocessConfig,
    asks: u64,
}

#[derive(Debug, PartialEq, Eq)]
enum CancelStep {
    Idle,
    Send,
    Wait,
    Kill,
}

enum Ended {
    Done,
    Interrupted { kill: bool },
    Failed(String),
}

fn opening_prompt(system: &str, prompt: &str) -> String {
    format!("{system}\n\n{prompt}")
}

fn next_prompt(opening: &mut Option<String>, prompt: &str) -> String {
    match opening.take() {
        Some(system) => opening_prompt(&system, prompt),
        None => prompt.to_string(),
    }
}

fn env_entries(entry: &Value) -> Vec<Value> {
    entry["env"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, value)| json!({"name": name, "value": value.as_str().unwrap_or_default()}))
        .collect()
}

pub fn acp_servers(mcp_config: &Value) -> Result<Vec<McpServer>, String> {
    mcp_config["mcpServers"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, entry)| entry["command"].is_string())
        .map(|(name, entry)| {
            let args = entry["args"].as_array().cloned().unwrap_or_default();
            serde_json::from_value(json!({
                "name": name,
                "command": entry["command"],
                "args": args,
                "env": env_entries(entry),
            }))
            .map_err(|error| format!("could not build the MCP server {name}: {error}"))
        })
        .collect()
}

fn notice_failure(text: &str) -> Option<String> {
    agy_notice(text).map(|notice| AgyNapError::Notice(notice).to_string())
}

fn settled(mut result: StreamingSubprocessResult) -> StreamingSubprocessResult {
    let text = extract_response_text(&result.stream_result.output_blocks).unwrap_or_default();
    if let Some(message) = notice_failure(&text) {
        result.exit_code = 1;
        result.stderr = message;
    }
    result
}

fn cancel_step(requested: bool, since_sent: Option<Duration>) -> CancelStep {
    match (requested, since_sent) {
        (false, _) => CancelStep::Idle,
        (true, None) => CancelStep::Send,
        (true, Some(elapsed)) if elapsed >= CANCEL_GRACE => CancelStep::Kill,
        (true, Some(_)) => CancelStep::Wait,
    }
}

impl ChatBackend for AgySlot {
    fn capabilities(&self) -> Caps {
        AGY
    }

    fn spawn(
        &self,
        launch: &ChatLaunch,
        io: &TurnIo,
    ) -> Result<Box<dyn ChatSession>, SubprocessError> {
        let live = AcpLive::open(AcpOpen {
            cwd: &self.cwd,
            servers: self.servers.clone(),
            model: Some(&self.model),
            effort: self.effort.as_deref(),
            policy: PermissionPolicy::Chat(self.policy.clone()),
        })
        .map_err(|message| SubprocessError::SpawnFailed(io::Error::other(message)))?;
        Ok(Box::new(AgySession {
            live: Some(live),
            opening: Some(strip_controls(launch.system).into_owned()),
            config: SpinnerSubprocessConfig {
                command: String::new(),
                timeout_secs: None,
                verbose_tools: VerboseToolsConfig::new(),
                show_tools: io.show_tools,
                terminated: io.terminated,
                sink: Arc::clone(&io.sink),
                gate: io.gate.clone(),
                interrupt: None,
                stdin: None,
            },
            asks: 0,
        }))
    }
}

fn answer_asks(gate: Option<&OutputGate>, pending: &mut Vec<(String, AnswerSender)>, deny: bool) {
    let Some(gate) = gate else { return };
    while let Some((ask, choice)) = gate.take_answer(deny) {
        if let Some(index) = pending
            .iter()
            .position(|(id, _)| *id == ask.content.request_id)
        {
            pending.remove(index).1.send(choice).ok();
        }
    }
}

impl AgySession {
    fn drive(
        &mut self,
        text: &str,
        cancel: &CancelToken,
        pump: &mut Pump,
    ) -> Result<Ended, SubprocessError> {
        let live = self.live.as_mut().ok_or(SubprocessError::Stale)?;
        live.prompt(text).map_err(|_| SubprocessError::Stale)?;
        let gate = self.config.gate.clone();
        let mut pending: Vec<(String, AnswerSender)> = Vec::new();
        let mut cancel_sent: Option<Instant> = None;
        loop {
            let requested = cancel.is_cancelled() || (self.config.terminated)();
            match cancel_step(requested, cancel_sent.map(|at| at.elapsed())) {
                CancelStep::Send => {
                    live.cancel();
                    cancel_sent = Some(Instant::now());
                }
                CancelStep::Kill => return Ok(Ended::Interrupted { kill: true }),
                CancelStep::Idle | CancelStep::Wait => {}
            }
            match live.next(POLL) {
                None if !live.is_alive() => {
                    return Ok(Ended::Failed("agy exited during the turn".to_string()))
                }
                None => {}
                Some(AcpEvent::Update(update)) => {
                    for event in agy_stream_events(&AgyEvent::Update(&update)) {
                        pump.event(event);
                    }
                }
                Some(AcpEvent::Ask(mut ask, sender)) => {
                    self.asks += 1;
                    let id = format!("acp-{}", self.asks);
                    ask.content.request_id.clone_from(&id);
                    let shown = cancel_sent.is_none()
                        && gate
                            .as_deref()
                            .is_some_and(|gate| gate.request_permission(ask, Classes::default()));
                    if shown {
                        pending.push((id, sender));
                    } else {
                        sender.send(Choice::Deny).ok();
                    }
                }
                Some(AcpEvent::Done { stop_reason, usage }) => {
                    for event in agy_stream_events(&AgyEvent::Done {
                        stop_reason,
                        usage: usage.as_ref(),
                    }) {
                        pump.event(event);
                    }
                    return Ok(if cancel_sent.is_some() {
                        Ended::Interrupted { kill: false }
                    } else {
                        Ended::Done
                    });
                }
                Some(AcpEvent::Failed(message)) => return Ok(Ended::Failed(message)),
            }
            answer_asks(gate.as_deref(), &mut pending, cancel_sent.is_some());
        }
    }
}

impl ChatSession for AgySession {
    fn turn(
        &mut self,
        prompt: &str,
        cancel: &CancelToken,
    ) -> Result<StreamingSubprocessResult, SubprocessError> {
        let text = next_prompt(&mut self.opening, prompt);
        let mut pump = Pump::start(&self.config, Display::Shown);
        let ended = self.drive(&text, cancel, &mut pump);
        if let Some(gate) = &self.config.gate {
            gate.cancel_permission(None);
        }
        match ended? {
            Ended::Done => Ok(settled(pump.finish_events(0))),
            Ended::Interrupted { kill } => {
                if kill {
                    self.live = None;
                }
                let partial = pump.finish_events(EXIT_CODE_INTERRUPTED);
                if cancel.is_cancelled() {
                    Ok(partial)
                } else {
                    Err(SubprocessError::Interrupted {
                        partial_result: Box::new(partial),
                    })
                }
            }
            Ended::Failed(message) => {
                self.live = None;
                Err(SubprocessError::OutputCaptureFailed(message))
            }
        }
    }

    fn ping(
        &mut self,
        _prompt: &str,
        _timeout: Duration,
    ) -> Result<StreamingSubprocessResult, SubprocessError> {
        Err(SubprocessError::OutputCaptureFailed(
            "agy sessions have no keepalive ping".to_string(),
        ))
    }

    fn is_alive(&mut self) -> bool {
        self.live.as_mut().is_some_and(AcpLive::is_alive)
    }

    fn discard(&mut self) {
        self.live = None;
    }
}
