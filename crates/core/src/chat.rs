use crate::chat_memory::SERVER_NAME;
use serde_json::json;
use std::path::Path;

pub const VIEW_LINES: usize = 200;

pub const SYSTEM_PROMPT: &str = "\
You are in a long-running chat with one person. The message you receive starts with the whole chat so far, compressed, oldest first, then the new message.

Lines that start with #<id> are summaries of older parts of the chat. The zoom tool opens one into its two halves. The recall tool searches the full chat by regex.

Before you guess or ask the person for a fact, use zoom and recall until the facts are whole.

The chat is logged automatically. Never claim to save, store, or remember anything manually.";

pub fn turn_prompt(view_lines: &[String], message: &str) -> String {
    format!("{}\n\nNew message:\n{message}", view_lines.join("\n"))
}

pub fn build_mcp_args(exe: &Path, store: &Path) -> Vec<String> {
    let config = json!({
        "mcpServers": {
            SERVER_NAME: {
                "command": exe.to_string_lossy(),
                "args": ["chat-mcp", "--store", store.to_string_lossy()]
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

    #[test]
    fn turn_prompt_puts_view_before_message() {
        let lines = vec!["0 user: hi".to_string(), "#1-2 s".to_string()];
        assert_eq!(
            turn_prompt(&lines, "yo"),
            "0 user: hi\n#1-2 s\n\nNew message:\nyo"
        );
    }

    #[test]
    fn mcp_args_name_only_the_chat_server() {
        let args = build_mcp_args(Path::new("/bin/ralph"), Path::new("/s"));
        assert_eq!(args[2], "--strict-mcp-config");
        let config: serde_json::Value = serde_json::from_str(&args[1]).unwrap();
        let servers = config["mcpServers"].as_object().unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(
            servers[SERVER_NAME]["args"],
            json!(["chat-mcp", "--store", "/s"])
        );
    }
}
