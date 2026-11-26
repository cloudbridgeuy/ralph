use super::{parse_status, status_tool, ReportStatus, SERVER_NAME};
use crate::mcp::{dispatch, string_arg, tool_result, Dispatch};
use crate::sanitize::strip_controls;
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Silent,
    Respond(Value),
    Report {
        id: Value,
        status: ReportStatus,
        text: String,
    },
    Status {
        id: Value,
        text: String,
    },
}

pub fn tool_definitions() -> Value {
    json!([
        {
            "name": "report",
            "description": "Send a message to the person who dispatched you. Call it at the end of every turn: when you finish, get blocked because only the person can unblock you, or need a decision. Then stop and wait for their answer. Each report costs a full chat turn, so never send progress updates; while waiting on your own subagents, checks or builds, call set_status and keep waiting in this turn.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "status": {"type": "string", "enum": ReportStatus::ALL.map(ReportStatus::as_str), "description": "done: task finished. blocked: you cannot go on and only the person can unblock you (not waiting on your own subagents, checks or builds). question: you need a decision only the person can make."},
                    "text": {"type": "string", "description": "Full summary: what you did and what you found, or what blocks you, or what you need decided."}
                },
                "required": ["status", "text"]
            }
        },
        status_tool()
    ])
}

fn parse_call(id: Value, name: &str, args: &Value) -> Result<Outcome, String> {
    match name {
        "report" => {
            let raw = string_arg(args, "status")?;
            let status = ReportStatus::parse(&raw)
                .ok_or_else(|| format!("Status '{raw}' must be done, blocked or question."))?;
            Ok(Outcome::Report {
                id,
                status,
                text: strip_controls(&string_arg(args, "text")?).into_owned(),
            })
        }
        "set_status" => Ok(Outcome::Status {
            id,
            text: parse_status(args)?,
        }),
        other => Err(format!("Unknown tool '{other}'.")),
    }
}

pub fn handle_request(request: &Value) -> Outcome {
    match dispatch(SERVER_NAME, &tool_definitions(), request) {
        Dispatch::Silent => Outcome::Silent,
        Dispatch::Respond(v) => Outcome::Respond(v),
        Dispatch::Call { id, name, args } => parse_call(id.clone(), &name, &args)
            .unwrap_or_else(|message| Outcome::Respond(tool_result(id, &message, true))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_text_strips_nul_and_other_controls() {
        let request = json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": {"name": "report", "arguments": {
            "status": "done",
            "text": "a\0b\u{0001}c\n\td`e\u{007f}"
        }}});
        assert_eq!(
            handle_request(&request),
            Outcome::Report {
                id: json!(3),
                status: ReportStatus::Done,
                text: "abc\n\td`e".into()
            }
        );
    }

    #[test]
    fn report_call_yields_status_and_text() {
        let request = json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {"name": "report", "arguments": {"status": "done", "text": "hello"}}});
        assert_eq!(
            handle_request(&request),
            Outcome::Report {
                id: json!(3),
                status: ReportStatus::Done,
                text: "hello".into()
            }
        );
    }

    #[test]
    fn report_schema_and_parser_accept_only_known_statuses() {
        assert_eq!(
            tool_definitions()[0]["inputSchema"]["properties"]["status"]["enum"],
            json!(["done", "blocked", "question"])
        );
        for status in ReportStatus::ALL {
            assert_eq!(ReportStatus::parse(status.as_str()), Some(status));
        }
        let request = json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {"name": "report", "arguments": {"status": "ok", "text": "hello"}}});
        let Outcome::Respond(v) = handle_request(&request) else {
            panic!("expected error response");
        };
        assert_eq!(v["result"]["isError"], true);
    }

    #[test]
    fn report_without_text_is_tool_error() {
        let request = json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {"name": "report", "arguments": {"status": "done"}}});
        let Outcome::Respond(v) = handle_request(&request) else {
            panic!("expected error response");
        };
        assert_eq!(v["result"]["isError"], true);
    }

    fn call(name: &str, arguments: Value) -> Outcome {
        handle_request(&json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": {"name": name, "arguments": arguments}}))
    }

    #[test]
    fn set_status_is_listed_next_to_report() {
        let names: Vec<_> = tool_definitions()
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, ["report", "set_status"]);
    }

    #[test]
    fn set_status_call_yields_the_cleaned_text() {
        assert_eq!(
            call("set_status", json!({"text": " Reading\nthe tests\0 "})),
            Outcome::Status {
                id: json!(4),
                text: "Reading the tests".into()
            }
        );
    }

    #[test]
    fn bad_set_status_calls_are_tool_errors() {
        for arguments in [
            json!({}),
            json!({"text": "  "}),
            json!({"text": "a".repeat(121)}),
        ] {
            let Outcome::Respond(v) = call("set_status", arguments) else {
                panic!("expected error response");
            };
            assert_eq!(v["result"]["isError"], true);
        }
    }

    #[test]
    fn report_tool_description_defines_blocked_as_needs_person() {
        let defs = tool_definitions();
        let report = &defs[0];
        let description = report["description"].as_str().unwrap();
        let status_desc = report["inputSchema"]["properties"]["status"]["description"]
            .as_str()
            .unwrap();
        assert!(
            description.contains("only the person can unblock you"),
            "{description}"
        );
        assert!(
            description.contains("subagents, checks or builds")
                && description.contains("set_status"),
            "{description}"
        );
        assert!(
            status_desc.contains("only the person can unblock you")
                && status_desc.contains("subagents, checks or builds"),
            "{status_desc}"
        );
    }
}
