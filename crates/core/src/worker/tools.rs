use super::{parse_status, status_tool, SERVER_NAME};
use crate::mcp::{dispatch, string_arg, tool_result, Dispatch};
use crate::sanitize::strip_controls;
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportStatus {
    Done,
    Blocked,
    Question,
}

impl ReportStatus {
    pub const ALL: [ReportStatus; 3] = [Self::Done, Self::Blocked, Self::Question];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Blocked => "blocked",
            Self::Question => "question",
        }
    }

    fn parse(status: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|known| known.as_str() == status)
            .ok_or_else(|| format!("Status '{status}' must be done, blocked or question."))
    }
}

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
            "description": "Send a message to the person who dispatched you. Call it at the end of every turn: when you finish, get blocked, or need a decision. Then stop and wait for their answer. Each report costs a full chat turn, so never send progress updates.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "status": {"type": "string", "enum": ReportStatus::ALL.map(ReportStatus::as_str), "description": "done: task finished. blocked: you cannot go on. question: you need a decision."},
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
        "report" => Ok(Outcome::Report {
            id,
            status: ReportStatus::parse(&string_arg(args, "status")?)?,
            text: strip_controls(&string_arg(args, "text")?).into_owned(),
        }),
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
            assert_eq!(ReportStatus::parse(status.as_str()), Ok(status));
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
}
