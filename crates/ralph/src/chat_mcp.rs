use crate::chat_memory::Store;
use crate::watch_requests;
use crate::workers::Workers;
use ralph_core::chat_memory::{finish_call, handle_request, Outcome};
use ralph_core::mcp::tool_result;
use ralph_core::watch_request::queued_message;
use ralph_core::worker::{command_allowed, permission_reply_allowed, Command};
use serde_json::Value;
use std::io::{self, BufRead, Write};

fn run_command(store: &Store, workers: &Workers, command: &Command) -> Result<String, String> {
    let pending = store
        .pending_turn()
        .map_err(|e| format!("Could not read chat memory: {e}"))?;
    command_allowed(command, pending)?;
    if let Command::Reply { id, .. } = command {
        if workers.awaits_permission(id) {
            permission_reply_allowed(pending)?;
        }
    }
    match command {
        Command::Spawn { id, task, harness } => workers.spawn(id, task, harness),
        Command::Reply { id, text } => workers.reply(id, text),
        Command::Close { id, abandon, force } => workers.close(id, *abandon, *force),
        Command::Watch(request) => {
            watch_requests::queue(store.dir(), request).map(|()| queued_message(&request.id))
        }
    }
    .map_err(|e| e.to_string())
}

pub fn run_stdio_server(
    store: &Store,
    workers: &Workers,
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
            Outcome::Command { id, command } => match run_command(store, workers, &command) {
                Ok(text) => tool_result(id, &text, false),
                Err(message) => tool_result(id, &message, true),
            },
        };
        writeln!(output, "{response}")?;
        output.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::chat_memory::Trigger;

    fn run(store: &Store, input: &str) -> Vec<Value> {
        let mut out = Vec::new();
        let workers = Workers::new("/s".into(), "/bin/ralph".into(), "auto".into(), None);
        run_stdio_server(store, &workers, input.as_bytes(), &mut out).unwrap();
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
        let zoom = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"zoom","arguments":{"id":"0"}}}"#;
        let empty = run(&store, &format!("{zoom}\n"));
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
        let lines = run(&store, &input);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1]["result"]["isError"], false);
        assert!(lines[1]["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("hello raw"));
    }

    #[test]
    fn watch_call_queues_one_request_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        store.set_pending_turn(Trigger::User).unwrap();
        let call = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"watch","arguments":{"id":"ci","command":"gh run watch 1","follow_up":"report","persistent":true,"timeout_secs":60}}}"#;
        let lines = run(&store, &format!("{call}\n"));
        assert_eq!(lines[0]["result"]["isError"], false);
        let queued = watch_requests::take_all(dir.path());
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].command, "gh run watch 1");
        assert!(queued[0].persistent);
        assert_eq!(queued[0].timeout_secs, Some(60));
    }

    #[test]
    fn watch_is_refused_on_a_worker_turn() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        store.set_pending_turn(Trigger::Worker).unwrap();
        let call = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"watch","arguments":{"id":"ci","command":"c","follow_up":"f"}}}"#;
        let lines = run(&store, &format!("{call}\n"));
        assert_eq!(lines[0]["result"]["isError"], true);
        assert!(watch_requests::take_all(dir.path()).is_empty());
    }

    #[test]
    fn spawn_worker_is_refused_on_a_worker_turn() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        std::fs::write(
            dir.path().join("log.jsonl"),
            "{\"role\":\"worker\",\"at\":\"t\",\"text\":\"merge?\"}\n",
        )
        .unwrap();
        store.set_pending_turn(Trigger::Worker).unwrap();
        let call = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"spawn_worker","arguments":{"id":"A","task":"merge"}}}"#;
        let lines = run(&store, &format!("{call}\n"));
        assert_eq!(lines[0]["result"]["isError"], true);
        assert!(lines[0]["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Only the person"));
    }
}
