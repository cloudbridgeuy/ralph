use super::range::{parse_read_args, RangeRequest};
use super::search::{parse_search_args, SearchRequest};
use super::{Node, SERVER_NAME};
use crate::changes::{changes_tool_definition, parse_changes_args, ChangeRequest, ScopeQuery};
use crate::chat_help::{config_topic, help_tool, help_topic};
use crate::chat_send_it::{propose_run_tool, step_result_tool};
use crate::chat_socket::is_socket_tool;
use crate::chat_thread::{scope_arg, split_scope, Scope};
use crate::mcp::{dispatch, string_arg, tool_result, Dispatch};
use crate::watch_tools::{watch_list_definition, watch_stop_definition};
use crate::worker::command_definitions;
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tool {
    Zoom(Node),
    Recall(String),
    Read(RangeRequest),
    Search(SearchRequest),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryCall {
    pub scope: Scope,
    pub tool: Tool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Silent,
    Respond(Value),
    Call {
        id: Value,
        call: MemoryCall,
    },
    Socket {
        id: Value,
        name: String,
        args: Value,
    },
    Changes {
        id: Value,
        request: ChangeRequest<ScopeQuery>,
    },
    ConfigHelp {
        id: Value,
        section: Option<String>,
    },
}

pub fn tool_definitions() -> Value {
    let mut tools = memory_tool_definitions();
    if let Some(list) = tools.as_array_mut() {
        list.extend(command_definitions());
        list.push(watch_list_definition());
        list.push(watch_stop_definition());
        list.push(propose_run_tool());
        list.push(step_result_tool());
        list.push(help_tool());
    }
    tools
}

fn memory_tool_definitions() -> Value {
    json!([
        {
            "name": "zoom",
            "description": "Open one node of the chat memory. A block id like 4-7 shows its two halves; a single index like 5 shows the raw message. Copy ids from the memory view. In a thread, its own ids carry the thread name (auth:4-7); ids without a prefix are the main chat before the fork.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "Node id such as 16-31, 5, or auth:5 for thread auth."}
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
                    "pattern": {"type": "string", "description": "Regular expression to search for."},
                    "thread": {"type": "string", "description": "Search main or a named thread instead of this chat's own messages."}
                },
                "required": ["pattern"]
            }
        },
        {
            "name": "read",
            "description": "Read a flat range of the chat in order, a page at a time. Each message shows as `#<id> · <role> · <date> · <n> chars` and then its text. mode raw (default) gives the original text; summary gives the leaf summary for long messages (`[not summarized]` when none exists). A message too long for the page is split into parts marked `chars a-b`. When more remains the page ends with `next cursor: <cursor>`; pass it back as cursor, with the same id and mode, for the next page. A cursor only works in the mode that issued it. Messages added after the first call never appear in later pages.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "Inclusive range such as 16-40, or one index such as 5. Prefix a thread name (auth:16-40) for a thread's messages."},
                    "mode": {"type": "string", "enum": ["raw", "summary"], "description": "raw (default) or summary."},
                    "budget": {"type": "integer", "description": "Page size in tokens, 1 to 16384. Default 4096."},
                    "cursor": {"type": "string", "description": "The `next cursor:` value from the previous page."}
                },
                "required": ["id"]
            }
        },
        {
            "name": "search",
            "description": "Ranked search over the original chat messages. Unlike recall (a regex, every hit newest first), search scores messages by how well they match bare words, \"quoted phrases\" (the words must appear together, in order), and identifiers such as GUZ-709 or foo_bar (matched whole: GUZ-709 does not match GUZ-7090). A plain word also matches inside a joined one: rate finds rate-limited. Rarer words weigh more; ties go to the newer message. Each hit is `#<id> · <role> · <timestamp> · <snippet>`. When more hits remain the page ends with `next cursor: <cursor>`; pass it back as cursor with the same query and filters. Call read with an id for the full text.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Words, \"quoted phrases\", and identifiers. A message needs every phrase and at least one word or phrase."},
                    "role": {"type": "string", "enum": ["user", "assistant", "worker"], "description": "Only messages from this role."},
                    "tag": {"type": "string", "enum": ["rule", "decision", "approval"], "description": "Only messages tagged this way. Tags exist only for messages written after tagging began."},
                    "since": {"type": "string", "description": "Earliest time: an RFC 3339 timestamp or a date like 2026-10-20."},
                    "until": {"type": "string", "description": "Latest time: an RFC 3339 timestamp or a date like 2026-10-20 (the whole day counts)."},
                    "limit": {"type": "integer", "description": "Hits per page, 1 to 50. Default 20."},
                    "cursor": {"type": "string", "description": "The `next cursor:` value from the previous page."},
                    "thread": {"type": "string", "description": "Search main or a named thread instead of this chat's own messages."}
                },
                "required": ["query"]
            }
        },
        changes_tool_definition(),
        {
            "name": "workers",
            "description": "List this chat's live workers, one line each: `<id> <harness> <state> <branch> <report>`. State is running, busy, idle, permission, a worker's last report, or gone. Report is the worker's status and its age when newer than its last report, else the last report kind and its age; unread inbox reports count. It is the same report /workers shows. Read-only; takes no arguments. Use it for worker status; never inspect system processes.",
            "inputSchema": {"type": "object", "properties": {}}
        }
    ])
}

pub fn parse_tool_call(name: &str, args: &Value) -> Result<MemoryCall, String> {
    let id = || string_arg(args, "id");
    let (scope, tool) = match name {
        "zoom" => {
            let id = id()?;
            let (scope, rest) = split_scope(&id)?;
            (scope, Node::parse(rest).map(Tool::Zoom)?)
        }
        "read" => {
            let id = id()?;
            let (scope, rest) = split_scope(&id)?;
            let mut args = args.clone();
            args["id"] = json!(rest);
            (scope, parse_read_args(&args).map(Tool::Read)?)
        }
        "recall" => (scope_arg(args)?, Tool::Recall(string_arg(args, "pattern")?)),
        "search" => (scope_arg(args)?, parse_search_args(args).map(Tool::Search)?),
        other => return Err(format!("Unknown tool '{other}'.")),
    };
    Ok(MemoryCall { scope, tool })
}

pub fn history_tool(
    name: &str,
    words: &str,
    cursor: Option<&str>,
    tag: Option<&str>,
) -> Result<MemoryCall, String> {
    let words = words.trim();
    if tag.is_some() && name != "search" {
        return Err("Only search takes a tag.".to_string());
    }
    let args = match name {
        "read" => {
            let mut parts = words.split_whitespace();
            let (id, mode) = (parts.next(), parts.next());
            if parts.next().is_some() {
                return Err("read takes an id and an optional mode, raw or summary.".to_string());
            }
            json!({"id": id, "mode": mode, "cursor": cursor})
        }
        "search" => json!({"query": words, "cursor": cursor, "tag": tag}),
        _ if cursor.is_some() => return Err(format!("{name} takes no cursor.")),
        "zoom" => json!({"id": words}),
        "recall" => json!({"pattern": words}),
        _ => Value::Null,
    };
    parse_tool_call(name, &args)
}

pub fn handle_request(request: &Value) -> Outcome {
    match dispatch(SERVER_NAME, &tool_definitions(), request) {
        Dispatch::Silent => Outcome::Silent,
        Dispatch::Respond(v) => Outcome::Respond(v),
        Dispatch::Call { id, name, args } if is_socket_tool(&name) => {
            Outcome::Socket { id, name, args }
        }
        Dispatch::Call { id, name, args } if name == "help" => {
            let topic = args.get("topic").and_then(Value::as_str);
            match config_topic(topic) {
                Some(section) => Outcome::ConfigHelp {
                    id,
                    section: section.map(str::to_string),
                },
                None => Outcome::Respond(finish_call(id, help_topic(topic))),
            }
        }
        Dispatch::Call { id, name, args } if name == "changes" => match parse_changes_args(&args) {
            Ok(request) => Outcome::Changes { id, request },
            Err(message) => Outcome::Respond(tool_result(id, &message, true)),
        },
        Dispatch::Call { id, name, args } => match parse_tool_call(&name, &args) {
            Ok(call) => Outcome::Call { id, call },
            Err(message) => Outcome::Respond(tool_result(id, &message, true)),
        },
    }
}

pub fn finish_call(id: Value, result: Result<String, String>) -> Value {
    match result {
        Ok(text) => tool_result(id, &text, false),
        Err(message) => tool_result(id, &message, true),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_memory::fixtures::entry;
    use crate::chat_memory::{read_range, recall, zoom, Entry, Read, Tree, Window};

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

    fn run(entries: &[Entry], tree: &Tree, tool: &Tool) -> Result<String, String> {
        let window = Window::full(entries);
        match tool {
            Tool::Zoom(node) => zoom(&window, tree, *node).map(|lines| lines.join("\n")),
            Tool::Recall(pattern) => recall(entries, pattern),
            Tool::Read(request) => match read_range(&window, tree, request)? {
                Read::Page(page) => Ok(page.render()),
                Read::More => panic!("a full window never needs more"),
            },
            Tool::Search(_) => panic!("search runs over an archive"),
        }
    }

    fn finish(entries: &[Entry], tree: &Tree, name: &str, args: Value) -> Value {
        match call(name, args) {
            Outcome::Call { id, call } => finish_call(id, run(entries, tree, &call.tool)),
            Outcome::Respond(v) => v,
            Outcome::Silent
            | Outcome::Socket { .. }
            | Outcome::Changes { .. }
            | Outcome::ConfigHelp { .. } => {
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
                "read",
                "search",
                "changes",
                "workers",
                "spawn_worker",
                "reply",
                "close_worker",
                "watch",
                "schedule",
                "watch_list",
                "watch_stop",
                "propose_run",
                "step_result",
                "help"
            ]
        );
    }

    #[test]
    fn workers_takes_no_arguments_and_routes_to_the_socket() {
        assert_eq!(
            call("workers", json!({})),
            Outcome::Socket {
                id: json!(7),
                name: "workers".into(),
                args: json!({})
            }
        );
    }

    #[test]
    fn help_answers_without_the_store() {
        let Outcome::Respond(v) = call("help", json!({"topic": "schedule"})) else {
            panic!("help must answer directly");
        };
        assert_eq!(v["result"]["isError"], false);
        assert!(text_of(&v).starts_with("/schedule\n"));
        let Outcome::Respond(v) = call("help", json!({"topic": "zap"})) else {
            panic!("help must answer directly");
        };
        assert_eq!(v["result"]["isError"], true);
    }

    #[test]
    fn help_routes_config_topics_to_the_shell() {
        assert_eq!(
            call("help", json!({"topic": "config"})),
            Outcome::ConfigHelp {
                id: json!(7),
                section: None
            }
        );
        assert_eq!(
            call("help", json!({"topic": "config chat"})),
            Outcome::ConfigHelp {
                id: json!(7),
                section: Some("chat".into())
            }
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
    fn read_pages_a_range_and_reports_errors() {
        let entries: Vec<Entry> = (0..4).map(|i| entry(i, 5)).collect();
        let tree = Tree::new();
        let v = finish(
            &entries,
            &tree,
            "read",
            json!({"id": "1-2", "budget": 4096}),
        );
        assert_eq!(v["result"]["isError"], false);
        let text = text_of(&v);
        assert!(text.contains("#1 · assistant") && text.contains("m2 xxxxx"));
        assert!(text.ends_with("(end of range)"));
        let paged = finish(&entries, &tree, "read", json!({"id": "0-3", "budget": 1}));
        assert!(text_of(&paged).contains("next cursor: "));
        for args in [
            json!({"id": "3-1"}),
            json!({"id": "9"}),
            json!({"id": "0", "mode": "x"}),
        ] {
            assert_eq!(
                finish(&entries, &tree, "read", args)["result"]["isError"],
                true
            );
        }
    }

    #[test]
    fn search_call_parses_or_reports_a_tool_error() {
        let Outcome::Call { call: parsed, .. } =
            call("search", json!({"query": "GUZ-709", "limit": 5}))
        else {
            panic!("expected call")
        };
        assert!(matches!(parsed.tool, Tool::Search(_)));
        let Outcome::Respond(v) = call("search", json!({"query": "x", "since": "soon"})) else {
            panic!("expected respond")
        };
        assert_eq!(v["result"]["isError"], true);
    }

    #[test]
    fn ids_and_the_thread_argument_carry_a_scope() {
        use crate::chat_thread::{Scope, ThreadName};
        let auth = Scope::Thread(ThreadName::parse("auth").unwrap());
        let scoped = |name: &str, args: Value| parse_tool_call(name, &args).unwrap();
        let zoom = scoped("zoom", json!({"id": "#auth:4-7"}));
        assert_eq!(zoom.scope, auth);
        assert_eq!(zoom.tool, Tool::Zoom(Node::new(4, 8).unwrap()));
        let read = scoped("read", json!({"id": "auth:2-3"}));
        assert!(matches!(
            read.tool,
            Tool::Read(RangeRequest { from: 2, to: 3, .. })
        ));
        assert_eq!(read.scope, auth);
        assert_eq!(scoped("zoom", json!({"id": "5"})).scope, Scope::Main);
        assert_eq!(scoped("recall", json!({"pattern": "x"})).scope, Scope::Own);
        assert_eq!(
            scoped("search", json!({"query": "x", "thread": "main"})).scope,
            Scope::Main
        );
        assert!(parse_tool_call("zoom", &json!({"id": "Bad:1"})).is_err());
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
