use crate::chat::changes::run_changes;
use crate::chat::memory::Store;
use crate::chat::socket::forward;
use ralph_core::chat_memory::{finish_call, handle_request, Outcome, Tool};
use ralph_core::chat_socket::SocketRequest;
use ralph_core::mcp::tool_result;
use serde_json::Value;
use std::io::{self, BufRead, Write};
use std::path::Path;

pub fn run_stdio_server(
    store: &Store,
    socket: &Path,
    input: impl BufRead,
    mut output: impl Write,
) -> io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                crate::warn::warn(format!("ignoring malformed JSON-RPC line: {e}"));
                continue;
            }
        };
        let response = match handle_request(&request) {
            Outcome::Silent => continue,
            Outcome::Respond(v) => v,
            Outcome::Call { id, tool } => match memory_call(store, &tool) {
                Ok(result) => finish_call(id, result),
                Err(e) => tool_result(id, &format!("Could not read chat memory: {e}"), true),
            },
            Outcome::Changes { id, request } => {
                let result = std::env::current_dir()
                    .map_err(|e| format!("Could not read the working directory: {e}"))
                    .and_then(|cwd| run_changes(store.dir(), &cwd, &request));
                match result {
                    Ok(text) => tool_result(id, &text, false),
                    Err(message) => tool_result(id, &message, true),
                }
            }
            Outcome::Worker { id, name, args } => {
                match forward(socket, &SocketRequest { tool: name, args }) {
                    Ok(reply) => tool_result(id, &reply.text, reply.is_error),
                    Err(message) => tool_result(id, &message, true),
                }
            }
        };
        writeln!(output, "{response}")?;
        output.flush()?;
    }
    Ok(())
}

fn memory_call(store: &Store, tool: &Tool) -> io::Result<Result<String, String>> {
    match tool {
        Tool::Zoom(node) => store.zoom(*node),
        Tool::Recall(pattern) => store.recall(pattern),
        Tool::Read(request) => store.read(request),
        Tool::Search(request) => store.search(request),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(store: &Store, socket: &Path, input: &str) -> Vec<Value> {
        let mut out = Vec::new();
        run_stdio_server(store, socket, input.as_bytes(), &mut out).unwrap();
        String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn stdio_server_zooms_to_raw_message_and_reloads_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let socket = dir.path().join("chat-1.sock");
        let zoom = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"zoom","arguments":{"id":"0"}}}"#;
        let empty = run(&store, &socket, &format!("{zoom}\n"));
        assert_eq!(empty[0]["result"]["isError"], true);

        std::fs::write(
            dir.path().join("log.jsonl"),
            "{\"role\":\"user\",\"at\":\"t\",\"text\":\"hello raw\"}\n",
        )
        .unwrap();
        let input = format!(
            "{}\n{}\n{zoom}\n",
            r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#
        );
        let lines = run(&store, &socket, &input);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1]["result"]["isError"], false);
        assert!(lines[1]["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("hello raw"));
    }

    fn text_of(response: &Value) -> String {
        response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn call(name: &str, arguments: Value) -> String {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": name, "arguments": arguments}
        });
        format!("{request}\n")
    }

    #[test]
    fn search_and_recall_read_the_log_under_the_shared_lock() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let socket = dir.path().join("chat-1.sock");
        let log = [
            "{\"role\":\"user\",\"at\":\"2026-10-20T10:00:00+00:00\",\"text\":\"ship GUZ-7090 first\"}",
            "{\"role\":\"assistant\",\"at\":\"2026-10-20T11:00:00+00:00\",\"text\":\"then ship GUZ-709\"}",
        ]
        .join("\n");
        std::fs::write(dir.path().join("log.jsonl"), format!("{log}\n")).unwrap();
        let input = [
            call("search", serde_json::json!({"query": "GUZ-709"})),
            call("recall", serde_json::json!({"pattern": "ship"})),
            call(
                "search",
                serde_json::json!({"query": "ship", "role": "user"}),
            ),
            call(
                "search",
                serde_json::json!({"query": "ship", "cursor": "1.2.00"}),
            ),
        ]
        .concat();
        let lines = run(&store, &socket, &input);
        let search = text_of(&lines[0]);
        assert!(
            search.starts_with("#1 · assistant · 2026-10-20T11:00:00+00:00 · then ship GUZ-709")
        );
        assert!(!search.contains("#0 "));
        let recall = text_of(&lines[1]);
        assert!(recall.find("#1 · ").unwrap() < recall.find("#0 · ").unwrap());
        assert!(recall.ends_with("(zoom <id> for the full text)"));
        assert!(text_of(&lines[2]).starts_with("#0 · user"));
        assert_eq!(lines[3]["result"]["isError"], true);
        assert!(dir.path().join("LOCK").exists());
    }

    #[test]
    fn search_and_recall_on_a_missing_store_find_nothing_and_create_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("absent");
        let store = Store::new(missing.clone());
        let socket = dir.path().join("chat-1.sock");
        let input = [
            call("search", serde_json::json!({"query": "x"})),
            call("recall", serde_json::json!({"pattern": "x"})),
        ]
        .concat();
        let lines = run(&store, &socket, &input);
        assert_eq!(text_of(&lines[0]), "no hits for x");
        assert_eq!(text_of(&lines[1]), "no hits for x");
        assert!(!missing.exists());
    }

    #[test]
    fn worker_tools_without_a_running_chat_return_a_tool_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let socket = dir.path().join("chat-1.sock");
        for name in ["spawn_worker", "reply", "close_worker", "watch"] {
            let call = format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"{name}","arguments":{{"id":"A"}}}}}}"#
            );
            let lines = run(&store, &socket, &format!("{call}\n"));
            assert_eq!(lines[0]["result"]["isError"], true);
            assert!(lines[0]["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("no running ralph chat"));
        }
    }
}
