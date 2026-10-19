use super::tree::{recall, zoom};
use super::{Entry, Node, Tree, SERVER_NAME};
use crate::changes::{changes_tool_definition, parse_changes_args, ChangeRequest, ScopeQuery};
use crate::chat_socket::is_worker_tool;
use crate::mcp::{dispatch, string_arg, tool_result, Dispatch};
use crate::worker::command_definitions;
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tool {
    Zoom(Node),
    Recall(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Silent,
    Respond(Value),
    Call {
        id: Value,
        tool: Tool,
    },
    Worker {
        id: Value,
        name: String,
        args: Value,
    },
    Changes {
        id: Value,
        request: ChangeRequest<ScopeQuery>,
    },
}

pub fn tool_definitions() -> Value {
    let mut tools = memory_tool_definitions();
    if let Some(list) = tools.as_array_mut() {
        list.extend(command_definitions());
    }
    tools
}

fn memory_tool_definitions() -> Value {
    json!([
        {
            "name": "zoom",
            "description": "Open one node of the chat memory. A block id like 4-7 shows its two halves; a single index like 5 shows the raw message. Copy ids from the memory view.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "Node id such as 16-31 or 5."}
                },
                "required": ["id"]
            }
        },
        {
            "name": "recall",
            "description": "Search the raw chat messages with a case-insensitive regex. Returns an index of every hit, newest first, one line each: `#<id> · <role> · <date> · <preview>`, where the preview is a short window around the match. At most 50 lines; narrow the pattern if more. Call zoom with an id for the full text.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "pattern": {"type": "string", "description": "Regular expression to search for."}
                },
                "required": ["pattern"]
            }
        },
        changes_tool_definition()
    ])
}

pub fn parse_tool_call(name: &str, args: &Value) -> Result<Tool, String> {
    match name {
        "zoom" => Node::parse(string_arg(args, "id")?.trim()).map(Tool::Zoom),
        "recall" => Ok(Tool::Recall(string_arg(args, "pattern")?)),
        other => Err(format!("Unknown tool '{other}'.")),
    }
}

pub fn handle_request(request: &Value) -> Outcome {
    match dispatch(SERVER_NAME, &tool_definitions(), request) {
        Dispatch::Silent => Outcome::Silent,
        Dispatch::Respond(v) => Outcome::Respond(v),
        Dispatch::Call { id, name, args } if is_worker_tool(&name) => {
            Outcome::Worker { id, name, args }
        }
        Dispatch::Call { id, name, args } if name == "changes" => match parse_changes_args(&args) {
            Ok(request) => Outcome::Changes { id, request },
            Err(message) => Outcome::Respond(tool_result(id, &message, true)),
        },
        Dispatch::Call { id, name, args } => match parse_tool_call(&name, &args) {
            Ok(tool) => Outcome::Call { id, tool },
            Err(message) => Outcome::Respond(tool_result(id, &message, true)),
        },
    }
}

fn run_tool(entries: &[Entry], tree: &Tree, tool: &Tool) -> Result<String, String> {
    match tool {
        Tool::Zoom(node) => zoom(entries, tree, *node).map(|lines| lines.join("\n")),
        Tool::Recall(pattern) => recall(entries, pattern),
    }
}

pub fn finish_call(id: Value, entries: &[Entry], tree: &Tree, tool: &Tool) -> Value {
    match run_tool(entries, tree, tool) {
        Ok(text) => tool_result(id, &text, false),
        Err(message) => tool_result(id, &message, true),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_memory::fixtures::entry;

    fn call(name: &str, args: Value) -> Outcome {
        handle_request(
            &json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": name, "arguments": args}}),
        )
    }

    fn text_of(v: &Value) -> String {
        v["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn finish(entries: &[Entry], tree: &Tree, name: &str, args: Value) -> Value {
        match call(name, args) {
            Outcome::Call { id, tool } => finish_call(id, entries, tree, &tool),
            Outcome::Respond(v) => v,
            Outcome::Silent | Outcome::Worker { .. } | Outcome::Changes { .. } => {
                panic!("not a memory call")
            }
        }
    }

    #[test]
    fn tools_list_names_memory_and_worker_tools() {
        let Outcome::Respond(v) = handle_request(&json!({"id": 1, "method": "tools/list"})) else {
            panic!("expected respond")
        };
        let names: Vec<&str> = v["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "zoom",
                "recall",
                "changes",
                "spawn_worker",
                "reply",
                "close_worker",
                "watch"
            ]
        );
    }

    #[test]
    fn zoom_walks_down_to_raw_message() {
        let entries: Vec<Entry> = (0..4).map(|i| entry(i, 5)).collect();
        let tree = Tree::new();
        let top = finish(&entries, &tree, "zoom", json!({"id": "0-3"}));
        assert_eq!(top["result"]["isError"], false);
        assert!(text_of(&top).contains("#0-1"));
        let leaf = finish(&entries, &tree, "zoom", json!({"id": "2"}));
        assert!(text_of(&leaf).contains("m2 xxxxx"));
    }

    #[test]
    fn recall_reports_hits() {
        let entries: Vec<Entry> = (0..4).map(|i| entry(i, 5)).collect();
        let v = finish(
            &entries,
            &Tree::new(),
            "recall",
            json!({"pattern": "M[13] "}),
        );
        let text = text_of(&v);
        assert!(text.contains("#3 · ") && text.contains("#1 · ") && !text.contains("#2 · "));
        assert!(text.find("#3 · ") < text.find("#1 · "));
        assert!(text.ends_with("(zoom <id> for the full text)"));
    }

    #[test]
    fn changes_call_parses_into_a_request() {
        let Outcome::Changes { id, request } = call("changes", json!({"format": "files"})) else {
            panic!("expected changes")
        };
        assert_eq!(id, json!(7));
        assert_eq!(request.format, crate::changes::ChangeFormat::Files);
    }

    #[test]
    fn changes_with_bad_args_is_a_tool_error() {
        let Outcome::Respond(v) = call("changes", json!({"last": 1, "ago": 2})) else {
            panic!("expected respond")
        };
        assert_eq!(v["result"]["isError"], true);
    }

    #[test]
    fn bad_inputs_are_tool_errors() {
        let entries: Vec<Entry> = (0..2).map(|i| entry(i, 5)).collect();
        let tree = Tree::new();
        for (name, args) in [
            ("zoom", json!({"id": "abc"})),
            ("zoom", json!({"id": "9"})),
            ("zoom", json!({})),
            ("recall", json!({"pattern": "("})),
            ("nope", json!({})),
        ] {
            assert_eq!(
                finish(&entries, &tree, name, args)["result"]["isError"],
                true
            );
        }
    }
}
