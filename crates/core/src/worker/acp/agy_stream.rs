use crate::stream::{
    AssistantEvent, AssistantMessage, ContentBlock, ResultEvent, StreamEvent, ToolResult, Usage,
    UserEvent, UserMessage,
};
use crate::worker::{agy_turn_usage, AgyUsage};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
    Other(String),
}

impl StopReason {
    pub fn parse(raw: &str) -> Self {
        match raw {
            "end_turn" => Self::EndTurn,
            "max_tokens" => Self::MaxTokens,
            "max_turn_requests" => Self::MaxTurnRequests,
            "refusal" => Self::Refusal,
            "cancelled" => Self::Cancelled,
            other => Self::Other(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::EndTurn => "end_turn",
            Self::MaxTokens => "max_tokens",
            Self::MaxTurnRequests => "max_turn_requests",
            Self::Refusal => "refusal",
            Self::Cancelled => "cancelled",
            Self::Other(raw) => raw,
        }
    }
}

pub enum AgyEvent<'a> {
    Update(&'a Value),
    Done {
        stop_reason: StopReason,
        usage: Option<&'a AgyUsage>,
    },
}

pub fn agy_stream_events(event: &AgyEvent) -> Vec<StreamEvent> {
    match event {
        AgyEvent::Update(update) => update_events(update),
        AgyEvent::Done { stop_reason, usage } => vec![StreamEvent::Result(ResultEvent {
            subtype: Some("success".into()),
            total_cost_usd: None,
            cost_usd: None,
            duration_ms: None,
            duration_api_ms: None,
            usage: usage.and_then(claude_usage),
            session_id: None,
            num_turns: None,
            result: None,
            terminal_reason: Some(stop_reason.as_str().into()),
            model_usage: Default::default(),
        })],
    }
}

#[derive(Default)]
pub struct AgyStream {
    held: String,
}

impl AgyStream {
    pub fn events(&mut self, event: &AgyEvent) -> Vec<StreamEvent> {
        if let AgyEvent::Update(update) = event {
            if update["sessionUpdate"].as_str() == Some("agent_message_chunk") {
                return self.hold(update["content"]["text"].as_str().unwrap_or_default());
            }
        }
        let breaks_text = match event {
            AgyEvent::Update(update) => matches!(
                update["sessionUpdate"].as_str(),
                Some("tool_call" | "tool_call_update")
            ),
            AgyEvent::Done { .. } => true,
        };
        let mut events = if breaks_text {
            self.release()
        } else {
            Vec::new()
        };
        events.extend(agy_stream_events(event));
        events
    }

    fn hold(&mut self, text: &str) -> Vec<StreamEvent> {
        self.held.push_str(text);
        let Some(end) = self.held.rfind('\n') else {
            return Vec::new();
        };
        let mut ready: String = self.held.drain(..=end).collect();
        ready.pop();
        vec![assistant(ContentBlock::Text { text: ready })]
    }

    fn release(&mut self) -> Vec<StreamEvent> {
        if self.held.is_empty() {
            return Vec::new();
        }
        vec![assistant(ContentBlock::Text {
            text: std::mem::take(&mut self.held),
        })]
    }
}

fn update_events(update: &Value) -> Vec<StreamEvent> {
    match update["sessionUpdate"].as_str() {
        Some("agent_message_chunk") => update["content"]["text"]
            .as_str()
            .map(|text| {
                assistant(ContentBlock::Text {
                    text: text.to_string(),
                })
            })
            .into_iter()
            .collect(),
        Some("tool_call") => vec![assistant(ContentBlock::ToolUse {
            id: update["toolCallId"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            name: claude_tool_name(
                ["name", "title", "kind"]
                    .iter()
                    .find_map(|key| update[*key].as_str())
                    .unwrap_or("tool"),
            ),
            input: update.get("rawInput").cloned().unwrap_or_else(|| json!({})),
        })],
        Some("tool_call_update") => {
            let status = update["status"].as_str();
            if !matches!(status, Some("completed" | "failed")) {
                return Vec::new();
            }
            vec![StreamEvent::User(UserEvent {
                message: UserMessage {
                    id: None,
                    content: vec![ToolResult {
                        result_type: Some("tool_result".into()),
                        tool_use_id: update["toolCallId"].as_str().map(str::to_string),
                        content: update["rawOutput"]["output"].as_str().map(str::to_string),
                        is_error: status == Some("failed"),
                    }],
                },
            })]
        }
        Some("usage_update") => update["used"]
            .as_u64()
            .map(|used| {
                vec![StreamEvent::Assistant(AssistantEvent {
                    message: AssistantMessage {
                        id: None,
                        content: Vec::new(),
                        model: None,
                        stop_reason: None,
                        usage: Some(Usage {
                            input_tokens: used,
                            output_tokens: 0,
                            cache_read_input_tokens: None,
                            cache_creation_input_tokens: None,
                            cache_creation: None,
                            output_tokens_details: None,
                        }),
                    },
                })]
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn assistant(block: ContentBlock) -> StreamEvent {
    StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: None,
            content: vec![block],
            model: None,
            stop_reason: None,
            usage: None,
        },
    })
}

fn claude_usage(raw: &AgyUsage) -> Option<Usage> {
    let usage = agy_turn_usage(raw).ok().flatten()?;
    Some(Usage {
        input_tokens: usage.input_uncached,
        output_tokens: usage.output,
        cache_read_input_tokens: Some(usage.cache_read),
        cache_creation_input_tokens: Some(usage.cache_writes.iter().map(|w| w.tokens).sum()),
        cache_creation: None,
        output_tokens_details: None,
    })
}

pub fn claude_tool_name(name: &str) -> String {
    name.strip_prefix("mcp_")
        .and_then(|rest| rest.split_once('_'))
        .filter(|(server, tool)| !server.is_empty() && !tool.is_empty())
        .map_or_else(
            || name.to_string(),
            |(server, tool)| format!("mcp__{server}__{tool}"),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(update: Value) -> StreamEvent {
        let mut events = agy_stream_events(&AgyEvent::Update(&update));
        assert_eq!(events.len(), 1, "{events:?}");
        events.remove(0)
    }

    fn blocks(event: StreamEvent) -> Vec<ContentBlock> {
        match event {
            StreamEvent::Assistant(assistant) => assistant.message.content,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn message_chunk_becomes_text() {
        let event = one(
            json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "hi"}}),
        );
        assert_eq!(blocks(event), [ContentBlock::Text { text: "hi".into() }]);
    }

    #[test]
    fn thought_chunk_adds_no_reply_text() {
        let update = json!({"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "hmm"}});
        assert!(agy_stream_events(&AgyEvent::Update(&update)).is_empty());
    }

    #[test]
    fn tool_call_becomes_tool_use_with_claude_name() {
        let event = one(
            json!({"sessionUpdate": "tool_call", "toolCallId": "c1", "name": "mcp_ralph-worker_report", "rawInput": {"text": "x"}}),
        );
        assert_eq!(
            blocks(event),
            [ContentBlock::ToolUse {
                id: "c1".into(),
                name: "mcp__ralph-worker__report".into(),
                input: json!({"text": "x"}),
            }]
        );
    }

    #[test]
    fn failed_update_becomes_error_tool_result() {
        let event = one(
            json!({"sessionUpdate": "tool_call_update", "toolCallId": "c1", "status": "failed", "rawOutput": {"output": "boom"}}),
        );
        let StreamEvent::User(user) = event else {
            panic!()
        };
        let [result] = user.message.content.as_slice() else {
            panic!()
        };
        assert_eq!(result.tool_use_id.as_deref(), Some("c1"));
        assert_eq!(result.content.as_deref(), Some("boom"));
        assert!(result.is_error);
    }

    #[test]
    fn completed_update_is_not_an_error_and_in_progress_is_silent() {
        let done = one(
            json!({"sessionUpdate": "tool_call_update", "toolCallId": "c1", "status": "completed"}),
        );
        assert!(matches!(done, StreamEvent::User(user) if !user.message.content[0].is_error));
        let running = json!({"sessionUpdate": "tool_call_update", "toolCallId": "c1", "status": "in_progress"});
        assert!(agy_stream_events(&AgyEvent::Update(&running)).is_empty());
    }

    #[test]
    fn agy_mcp_name_maps_to_claude_form() {
        assert_eq!(
            claude_tool_name("mcp_ralph-worker_set_status"),
            "mcp__ralph-worker__set_status"
        );
        assert_eq!(claude_tool_name("mcp_srv_a_b_c"), "mcp__srv__a_b_c");
        assert_eq!(claude_tool_name("run_command"), "run_command");
        assert_eq!(claude_tool_name("mcp_noTool"), "mcp_noTool");
    }

    #[test]
    fn done_with_usage_becomes_a_result_without_cost() {
        let usage = AgyUsage {
            total: 120,
            input: 100,
            output: 20,
            read: None,
            write: None,
            thought: None,
        };
        let events = agy_stream_events(&AgyEvent::Done {
            stop_reason: StopReason::EndTurn,
            usage: Some(&usage),
        });
        let [StreamEvent::Result(result)] = events.as_slice() else {
            panic!("{events:?}")
        };
        assert_eq!(result.get_cost_usd(), None);
        assert_eq!(result.terminal_reason.as_deref(), Some("end_turn"));
        let tokens = result
            .usage
            .as_ref()
            .map(|u| (u.input_tokens, u.output_tokens));
        assert_eq!(tokens, Some((100, 20)));
    }

    #[test]
    fn stop_reason_parses_known_and_keeps_other() {
        for (raw, parsed) in [
            ("end_turn", StopReason::EndTurn),
            ("max_tokens", StopReason::MaxTokens),
            ("max_turn_requests", StopReason::MaxTurnRequests),
            ("refusal", StopReason::Refusal),
            ("cancelled", StopReason::Cancelled),
            ("weird", StopReason::Other("weird".to_string())),
        ] {
            assert_eq!(StopReason::parse(raw), parsed);
            assert_eq!(parsed.as_str(), raw);
        }
    }

    #[test]
    fn usage_update_feeds_the_context_readout() {
        let event = one(json!({"sessionUpdate": "usage_update", "used": 53000, "size": 1000000}));
        let costs = crate::stream::extract_costs_from_events_or_default(&[event]);
        assert_eq!(costs.context_tokens, Some(53000));
    }

    #[test]
    fn usage_update_without_used_is_ignored() {
        let update = json!({"sessionUpdate": "usage_update", "size": 1000000});
        assert!(agy_stream_events(&AgyEvent::Update(&update)).is_empty());
    }
}

#[cfg(test)]
mod stream_tests {
    use super::*;

    fn text_of(events: Vec<StreamEvent>) -> Vec<String> {
        events
            .into_iter()
            .flat_map(|event| match event {
                StreamEvent::Assistant(assistant) => assistant.message.content,
                other => panic!("{other:?}"),
            })
            .map(|block| match block {
                ContentBlock::Text { text } => text,
                other => panic!("{other:?}"),
            })
            .collect()
    }

    fn chunk(text: &str) -> Value {
        json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}})
    }

    #[test]
    fn a_partial_line_waits_for_its_end() {
        let mut stream = AgyStream::default();
        assert!(stream.events(&AgyEvent::Update(&chunk("PING-"))).is_empty());
        let ready = stream.events(&AgyEvent::Update(&chunk("1\nrest")));
        assert_eq!(text_of(ready), ["PING-1"]);
    }

    #[test]
    fn a_tool_call_releases_the_held_line() {
        let mut stream = AgyStream::default();
        stream.events(&AgyEvent::Update(&chunk("done")));
        let call = json!({"sessionUpdate": "tool_call", "toolCallId": "t1", "name": "Read"});
        let events = stream.events(&AgyEvent::Update(&call));
        assert_eq!(events.len(), 2, "{events:?}");
        assert_eq!(text_of(events[..1].to_vec()), ["done"]);
    }

    #[test]
    fn a_usage_update_keeps_the_line_open() {
        let mut stream = AgyStream::default();
        stream.events(&AgyEvent::Update(&chunk("inter")));
        let usage = json!({"sessionUpdate": "usage_update", "used": 5});
        let events = stream.events(&AgyEvent::Update(&usage));
        assert!(text_of(events).is_empty());
        let ready = stream.events(&AgyEvent::Update(&chunk("rupt\n")));
        assert_eq!(text_of(ready), ["interrupt"]);
    }
}
