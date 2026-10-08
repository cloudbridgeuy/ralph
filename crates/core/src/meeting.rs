use crate::mcp::{dispatch, string_arg, tool_result, Dispatch};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;

pub const SERVER_NAME: &str = "ralph-meeting";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MeetingEvent {
    SendMessage {
        from: String,
        to: String,
        text: String,
    },
    AskHuman {
        from: String,
        question: String,
    },
    EndMeeting {
        from: String,
        summary: String,
    },
    Remember {
        from: String,
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MeetingReply {
    pub text: String,
    #[serde(default)]
    pub is_error: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeetingContext {
    pub caller: String,
    pub roster: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Silent,
    Respond(Value),
    Relay { id: Value, event: MeetingEvent },
}

pub fn tool_definitions() -> Value {
    json!([
        {
            "name": "send_message",
            "description": "Send a message to another meeting participant. After this call succeeds, end your turn immediately without further tool calls.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "to": {"type": "string", "description": "Name of the persona that receives the message."},
                    "text": {"type": "string", "description": "Message body."}
                },
                "required": ["to", "text"]
            }
        },
        {
            "name": "ask_human",
            "description": "Ask the human operator a question and wait for the answer.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "question": {"type": "string", "description": "Question for the human."}
                },
                "required": ["question"]
            }
        },
        {
            "name": "end_meeting",
            "description": "End the meeting with a final summary.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "summary": {"type": "string", "description": "Summary of the meeting outcome."}
                },
                "required": ["summary"]
            }
        },
        {
            "name": "remember",
            "description": "Save a durable note to the shared project office memory for future meetings. This does not end your turn; continue and finish with send_message or end_meeting.",
            "inputSchema": {
                "type": "object",
                "properties": {"text": {"type": "string", "description": "Durable project knowledge to retain."}},
                "required": ["text"]
            }
        }
    ])
}

pub fn parse_tool_call(
    ctx: &MeetingContext,
    name: &str,
    args: &Value,
) -> Result<MeetingEvent, String> {
    let from = ctx.caller.clone();
    match name {
        "send_message" => {
            let to = string_arg(args, "to")?;
            let text = string_arg(args, "text")?;
            if to == ctx.caller {
                return Err("You cannot send a message to yourself.".to_string());
            }
            if !ctx.roster.contains(&to) {
                return Err(format!(
                    "Unknown recipient '{to}'. Valid recipients: {}.",
                    ctx.roster
                        .iter()
                        .filter(|p| **p != ctx.caller)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            Ok(MeetingEvent::SendMessage { from, to, text })
        }
        "ask_human" => Ok(MeetingEvent::AskHuman {
            from,
            question: string_arg(args, "question")?,
        }),
        "end_meeting" => Ok(MeetingEvent::EndMeeting {
            from,
            summary: string_arg(args, "summary")?,
        }),
        "remember" => Ok(MeetingEvent::Remember {
            from,
            text: crate::meeting_memory::normalize_note(&string_arg(args, "text")?)?,
        }),
        other => Err(format!("Unknown tool '{other}'.")),
    }
}

pub fn finish_relay(id: Value, event: &MeetingEvent, reply: Result<MeetingReply, String>) -> Value {
    match (event, reply) {
        (_, Err(message)) => tool_result(id, &format!("Relay failed: {message}"), true),
        (_, Ok(reply)) if reply.is_error => tool_result(id, &reply.text, true),
        (MeetingEvent::SendMessage { to, .. }, Ok(_)) => tool_result(
            id,
            &format!(
                "Message delivered to {to}. End your turn now; do not make any further tool calls."
            ),
            false,
        ),
        (_, Ok(reply)) => tool_result(id, &reply.text, false),
    }
}

pub fn handle_request(ctx: &MeetingContext, request: &Value) -> Outcome {
    match dispatch(SERVER_NAME, &tool_definitions(), request) {
        Dispatch::Silent => Outcome::Silent,
        Dispatch::Respond(v) => Outcome::Respond(v),
        Dispatch::Call { id, name, args } => match parse_tool_call(ctx, &name, &args) {
            Ok(event) => Outcome::Relay { id, event },
            Err(message) => Outcome::Respond(tool_result(id, &message, true)),
        },
    }
}

pub fn build_mcp_args(exe: &Path, socket: &Path, caller: &str, personas: &[String]) -> Vec<String> {
    let config = json!({
        "mcpServers": {
            SERVER_NAME: {
                "command": exe.to_string_lossy(),
                "args": [
                    "meeting-mcp",
                    "--socket", socket.to_string_lossy(),
                    "--caller", caller,
                    "--personas", personas.join(",")
                ]
            }
        }
    });
    vec![
        "--mcp-config".to_string(),
        config.to_string(),
        "--strict-mcp-config".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn ctx() -> MeetingContext {
        MeetingContext {
            caller: "alice".into(),
            roster: vec!["alice".into(), "bob".into()],
        }
    }

    fn call(name: &str, args: Value) -> Outcome {
        handle_request(
            &ctx(),
            &json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": name, "arguments": args}}),
        )
    }

    fn respond(outcome: Outcome) -> Value {
        match outcome {
            Outcome::Respond(v) => v,
            other => panic!("expected Respond, got {other:?}"),
        }
    }

    #[test]
    fn initialize_echoes_protocol_version() {
        let v = respond(handle_request(
            &ctx(),
            &json!({"id": 1, "method": "initialize", "params": {"protocolVersion": "2024-11-05"}}),
        ));
        assert_eq!(v["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(v["result"]["serverInfo"]["name"], "ralph-meeting");
        assert!(v["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn tools_list_has_four_tools() {
        let v = respond(handle_request(
            &ctx(),
            &json!({"id": 1, "method": "tools/list"}),
        ));
        let names: Vec<&str> = v["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            ["send_message", "ask_human", "end_meeting", "remember"]
        );
    }

    #[test]
    fn notification_is_silent() {
        let outcome = handle_request(&ctx(), &json!({"method": "notifications/initialized"}));
        assert_eq!(outcome, Outcome::Silent);
    }

    #[test]
    fn unknown_method_is_protocol_error() {
        let v = respond(handle_request(&ctx(), &json!({"id": 3, "method": "nope"})));
        assert_eq!(v["error"]["code"], -32601);
    }

    #[test]
    fn valid_send_message_relays() {
        let outcome = call("send_message", json!({"to": "bob", "text": "hi"}));
        assert_eq!(
            outcome,
            Outcome::Relay {
                id: json!(7),
                event: MeetingEvent::SendMessage {
                    from: "alice".into(),
                    to: "bob".into(),
                    text: "hi".into()
                }
            }
        );
    }

    #[test]
    fn invalid_recipient_is_tool_error() {
        for to in ["alice", "carol"] {
            let v = respond(call("send_message", json!({"to": to, "text": "hi"})));
            assert_eq!(v["result"]["isError"], true);
        }
    }

    #[test]
    fn missing_argument_is_tool_error() {
        let v = respond(call("ask_human", json!({})));
        assert_eq!(v["result"]["isError"], true);
    }

    #[test]
    fn remember_uses_caller_normalizes_and_returns_append_errors() {
        let outcome = call("remember", json!({"from": "bob", "text": " first\n note "}));
        let Outcome::Relay { id, event } = outcome else {
            panic!("expected relay")
        };
        assert_eq!(
            event,
            MeetingEvent::Remember {
                from: "alice".into(),
                text: "first note".into()
            }
        );
        let ok = finish_relay(
            id.clone(),
            &event,
            Ok(MeetingReply {
                text: "Continue your turn".into(),
                ..MeetingReply::default()
            }),
        );
        assert_eq!(ok["result"]["content"][0]["text"], "Continue your turn");
        let err = finish_relay(
            id,
            &event,
            Ok(MeetingReply {
                text: "disk full".into(),
                is_error: true,
            }),
        );
        assert_eq!(err["result"]["isError"], true);
        assert_eq!(
            respond(call("remember", json!({"text": " \n"})))["result"]["isError"],
            true
        );
        let definition = tool_definitions();
        assert_eq!(definition[3]["inputSchema"]["required"], json!(["text"]));
    }

    #[test]
    fn finish_relay_send_message_tells_model_to_stop() {
        let event = MeetingEvent::SendMessage {
            from: "alice".into(),
            to: "bob".into(),
            text: "hi".into(),
        };
        let v = finish_relay(
            json!(1),
            &event,
            Ok(MeetingReply {
                text: "ok".into(),
                ..MeetingReply::default()
            }),
        );
        assert!(v["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("End your turn"));
        assert_eq!(v["result"]["isError"], false);
    }

    #[test]
    fn finish_relay_ask_human_returns_answer_and_errors_flag() {
        let event = MeetingEvent::AskHuman {
            from: "alice".into(),
            question: "q".into(),
        };
        let ok = finish_relay(
            json!(1),
            &event,
            Ok(MeetingReply {
                text: "42".into(),
                ..MeetingReply::default()
            }),
        );
        assert_eq!(ok["result"]["content"][0]["text"], "42");
        let err = finish_relay(json!(1), &event, Err("boom".into()));
        assert_eq!(err["result"]["isError"], true);
    }

    #[test]
    fn mcp_args_include_strict_flag_and_server_args() {
        let args = build_mcp_args(
            &PathBuf::from("/bin/ralph"),
            &PathBuf::from("/tmp/s.sock"),
            "alice",
            &["alice".to_string(), "bob".to_string()],
        );
        assert_eq!(args[0], "--mcp-config");
        assert_eq!(args[2], "--strict-mcp-config");
        let config: Value = serde_json::from_str(&args[1]).unwrap();
        let server = &config["mcpServers"]["ralph-meeting"];
        assert_eq!(server["command"], "/bin/ralph");
        assert_eq!(
            server["args"],
            json!([
                "meeting-mcp",
                "--socket",
                "/tmp/s.sock",
                "--caller",
                "alice",
                "--personas",
                "alice,bob"
            ])
        );
    }
}
