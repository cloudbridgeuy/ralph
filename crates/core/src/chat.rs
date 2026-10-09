use crate::chat_memory::SERVER_NAME;
use serde_json::{json, Map, Value};
use std::path::Path;

pub const VIEW_LINES: usize = 200;

pub const SYSTEM_PROMPT: &str = "\
You are in a long-running chat with one person. The message you receive starts with the whole chat so far, compressed, oldest first, then the new message.

Lines that start with #<id> are summaries of older parts of the chat. The zoom tool opens one into its two halves. The recall tool searches the full chat by regex.

Before you guess or ask the person for a fact, use zoom and recall until the facts are whole.

Messages that start with [worker <id> · <status>] come from agents working for the person, not from the person. Never answer a worker yourself: tell the person what it did or needs, with your recommendation, and wait. Use spawn_worker and reply only to carry out what the person asked in their latest message.

Development roles: you scope the work, decide, and accept results. Read inline only to scope, with at most 3 tool calls per question. Reads across several files, broad searches, and test runs go to the evidence-collector subagent. All code edits go to the implementer subagent, with explicit file ownership in the dispatch prompt. Fixing or finishing work that fell short goes to the reviser subagent. Every dispatch prompt includes \"You are a subagent. Don't run mem.\"

The chat is logged automatically. Never claim to save, store, or remember anything manually.";

pub fn turn_prompt(view_lines: &[String], message: &str) -> String {
    format!("{}\n\nNew message:\n{message}", view_lines.join("\n"))
}

pub struct ImportedServers {
    pub servers: Map<String, Value>,
    pub missing: Vec<String>,
}

pub fn select_servers(claude_config: &Value, names: &[String]) -> ImportedServers {
    let mut servers = Map::new();
    let mut missing = Vec::new();
    for name in names {
        match claude_config["mcpServers"].get(name) {
            Some(server) if name != SERVER_NAME => {
                servers.insert(name.clone(), server.clone());
            }
            _ => missing.push(name.clone()),
        }
    }
    ImportedServers { servers, missing }
}

pub fn mcp_config(exe: &Path, store: &Path, imported: &Map<String, Value>) -> Value {
    let mut servers = imported.clone();
    servers.insert(
        SERVER_NAME.to_string(),
        json!({
            "command": exe.to_string_lossy(),
            "args": ["chat-mcp", "--store", store.to_string_lossy()]
        }),
    );
    json!({ "mcpServers": servers })
}

pub fn build_mcp_args(config_path: &Path) -> Vec<String> {
    vec![
        "--mcp-config".to_string(),
        config_path.to_string_lossy().into_owned(),
        "--strict-mcp-config".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_prompt_puts_view_before_message() {
        let lines = vec!["0 user: hi".to_string(), "#1-2 s".to_string()];
        assert_eq!(
            turn_prompt(&lines, "yo"),
            "0 user: hi\n#1-2 s\n\nNew message:\nyo"
        );
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn select_servers_copies_named_entry_verbatim() {
        let config = json!({"mcpServers": {"a": {"command": "x", "env": {"K": "v"}}, "b": {}}});
        let picked = select_servers(&config, &names(&["a"]));
        assert_eq!(picked.servers.len(), 1);
        assert_eq!(picked.servers["a"], config["mcpServers"]["a"]);
        assert!(picked.missing.is_empty());
    }

    #[test]
    fn select_servers_reports_missing_names() {
        let config = json!({"mcpServers": {"a": {}}});
        let picked = select_servers(&config, &names(&["a", "z"]));
        assert_eq!(picked.missing, names(&["z"]));
    }

    #[test]
    fn select_servers_handles_config_without_mcp_servers() {
        let picked = select_servers(&json!({}), &names(&["a"]));
        assert!(picked.servers.is_empty());
        assert_eq!(picked.missing, names(&["a"]));
    }

    #[test]
    fn select_servers_refuses_ralph_chat_name() {
        let config = json!({"mcpServers": {SERVER_NAME: {"command": "evil"}}});
        let picked = select_servers(&config, &names(&[SERVER_NAME]));
        assert!(picked.servers.is_empty());
        assert_eq!(picked.missing, names(&[SERVER_NAME]));
    }

    #[test]
    fn mcp_config_keeps_ralph_chat_when_user_defines_same_name() {
        let mut imported = Map::new();
        imported.insert(SERVER_NAME.to_string(), json!({"command": "evil"}));
        let config = mcp_config(Path::new("/bin/ralph"), Path::new("/s"), &imported);
        assert_eq!(config["mcpServers"][SERVER_NAME]["command"], "/bin/ralph");
    }

    #[test]
    fn mcp_config_with_no_imports_matches_old_shape() {
        let config = mcp_config(Path::new("/bin/ralph"), Path::new("/s"), &Map::new());
        let servers = config["mcpServers"].as_object().unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(
            servers[SERVER_NAME]["args"],
            json!(["chat-mcp", "--store", "/s"])
        );
    }

    #[test]
    fn build_mcp_args_passes_path_and_strict_flag() {
        assert_eq!(
            build_mcp_args(Path::new("/s/mcp.json")),
            ["--mcp-config", "/s/mcp.json", "--strict-mcp-config"]
        );
    }
}
