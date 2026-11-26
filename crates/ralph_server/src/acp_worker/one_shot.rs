use super::live::{AcpEvent, AcpLive, AcpOpen, PermissionPolicy};
use super::*;
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

fn silence_error(expired: bool, timeout: Duration) -> OneShotError {
    if expired {
        OneShotError::TimedOut(timeout)
    } else {
        OneShotError::Failed("agy one-shot ended without a reply".to_string())
    }
}

pub struct OneShotRequest<'a> {
    pub cwd: &'a Path,
    pub prompt: &'a str,
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub timeout: Duration,
}

pub fn run(request: &OneShotRequest) -> Result<AcpOneShot, OneShotError> {
    let deadline = Instant::now() + request.timeout;
    let live = AcpLive::open(AcpOpen {
        cwd: request.cwd,
        servers: Vec::new(),
        model: request.model,
        effort: request.effort,
        policy: PermissionPolicy::RejectAll,
    })
    .map_err(OneShotError::Failed)?;
    live.prompt(request.prompt).map_err(OneShotError::Failed)?;
    let mut collected = Collected::default();
    let result = loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match live.next(remaining) {
            Some(AcpEvent::Update(update)) => fold(&mut collected, &update),
            Some(AcpEvent::Ask(..)) => {}
            Some(AcpEvent::Done { stop_reason, usage }) => {
                let Collected { text, tool_calls } = collected;
                break Ok(AcpOneShot {
                    text,
                    tool_calls,
                    usage,
                    stop_reason: stop_reason.as_str().to_string(),
                    model: live.model().to_string(),
                });
            }
            Some(AcpEvent::Failed(message)) => break Err(OneShotError::Failed(message)),
            None => break Err(silence_error(Instant::now() >= deadline, request.timeout)),
        }
    };
    live.close();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_receive_timeout_is_a_timed_out_error() {
        let timeout = Duration::from_secs(7);
        let error = silence_error(true, timeout);
        assert_eq!(error, OneShotError::TimedOut(timeout));
        assert_eq!(error.to_string(), "agy timed out after 7s");
    }

    #[test]
    fn a_dropped_sender_is_a_failure_not_a_timeout() {
        let error = silence_error(false, Duration::from_secs(7));
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
