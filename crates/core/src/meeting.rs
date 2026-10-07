use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;

pub const SERVER_NAME: &str = "ralph-meeting";
const DEFAULT_PROTOCOL_VERSION: &str = "2025-06-18";
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INVALID_REQUEST: i64 = -32600;

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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeetingReply {
    pub text: String,
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
        }
    ])
}

fn string_arg(args: &Value, key: &str) -> Result<String, String> {
    match args.get(key).and_then(Value::as_str) {
        Some(s) if !s.trim().is_empty() => Ok(s.to_string()),
        _ => Err(format!("Missing or empty string argument '{key}'.")),
    }
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
        other => Err(format!("Unknown tool '{other}'.")),
    }
}

fn success(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn failure(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn tool_result(id: Value, text: &str, is_error: bool) -> Value {
    success(
        id,
        json!({"content": [{"type": "text", "text": text}], "isError": is_error}),
    )
}

pub fn finish_relay(id: Value, event: &MeetingEvent, reply: Result<MeetingReply, String>) -> Value {
    match (event, reply) {
        (_, Err(message)) => tool_result(id, &format!("Relay failed: {message}"), true),
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
    let Some(method) = request.get("method").and_then(Value::as_str) else {
        return match request.get("id") {
            Some(id) => Outcome::Respond(failure(id.clone(), INVALID_REQUEST, "Invalid request")),
            None => Outcome::Silent,
        };
    };
    let Some(id) = request.get("id").cloned() else {
        return Outcome::Silent;
    };
    let params = request.get("params").cloned().unwrap_or(Value::Null);

    match method {
        "initialize" => {
            let version = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(DEFAULT_PROTOCOL_VERSION);
            Outcome::Respond(success(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": SERVER_NAME, "version": "0.1.0"}
                }),
            ))
        }
        "ping" => Outcome::Respond(success(id, json!({}))),
        "tools/list" => Outcome::Respond(success(id, json!({"tools": tool_definitions()}))),
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Outcome::Respond(failure(id, INVALID_PARAMS, "Missing tool name"));
            };
            let args = params.get("arguments").cloned().unwrap_or(Value::Null);
            match parse_tool_call(ctx, name, &args) {
                Ok(event) => Outcome::Relay { id, event },
                Err(message) => Outcome::Respond(tool_result(id, &message, true)),
            }
        }
        other => Outcome::Respond(failure(
            id,
            METHOD_NOT_FOUND,
            &format!("Method not found: {other}"),
        )),
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
    fn tools_list_has_three_tools() {
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
        assert_eq!(names, ["send_message", "ask_human", "end_meeting"]);
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
    fn finish_relay_send_message_tells_model_to_stop() {
        let event = MeetingEvent::SendMessage {
            from: "alice".into(),
            to: "bob".into(),
            text: "hi".into(),
        };
        let v = finish_relay(json!(1), &event, Ok(MeetingReply { text: "ok".into() }));
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
        let ok = finish_relay(json!(1), &event, Ok(MeetingReply { text: "42".into() }));
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
