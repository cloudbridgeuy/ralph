use crate::chat::memory::Store;
use crate::chat::socket::forward;
use ralph_core::chat_memory::{finish_call, handle_request, Outcome};
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
            Outcome::Call { id, tool } => match store.load() {
                Ok((entries, tree)) => finish_call(id, &entries, &tree, &tool),
                Err(e) => tool_result(id, &format!("Could not read chat memory: {e}"), true),
            },
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
