use super::shell_quote;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const SHELL_EVENT: &str = "beforeShellExecution";
const BRIDGE_ARGS: &str = " hook-bridge --socket ";

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum HookEvent {
    #[serde(rename = "beforeShellExecution")]
    BeforeShellExecution,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct HookInput {
    pub hook_event_name: HookEvent,
    pub command: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub conversation_id: String,
    #[serde(default)]
    pub generation_id: String,
    #[serde(default)]
    pub tool_use_id: String,
    #[serde(default)]
    pub workspace_roots: Vec<PathBuf>,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub cursor_version: String,
}

pub fn bridge_hook_command(exe: &Path, socket: &Path) -> String {
    format!(
        "{}{BRIDGE_ARGS}{}",
        shell_quote(&exe.to_string_lossy()),
        shell_quote(&socket.to_string_lossy())
    )
}

fn is_bridge_entry(entry: &Value) -> bool {
    entry["command"]
        .as_str()
        .is_some_and(|command| command.contains(BRIDGE_ARGS))
}

fn without_bridge(entries: &Value) -> Vec<Value> {
    entries
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| !is_bridge_entry(entry))
        .cloned()
        .collect()
}

pub fn add_bridge_hook(existing: Option<&str>, command: &str) -> String {
    let mut config = existing
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    if config.get("version").is_none() {
        config["version"] = json!(1);
    }
    if !config["hooks"].is_object() {
        config["hooks"] = json!({});
    }
    let mut entries = without_bridge(&config["hooks"][SHELL_EVENT]);
    entries.push(json!({ "command": command }));
    config["hooks"][SHELL_EVENT] = Value::Array(entries);
    config.to_string()
}

pub fn remove_bridge_hook(existing: &str) -> String {
    let Ok(mut config) = serde_json::from_str::<Value>(existing) else {
        return existing.to_string();
    };
    let Some(hooks) = config["hooks"].as_object_mut() else {
        return existing.to_string();
    };
    let Some(entries) = hooks.get(SHELL_EVENT).filter(|entries| entries.is_array()) else {
        return existing.to_string();
    };
    let kept = without_bridge(entries);
    if kept.is_empty() {
        hooks.remove(SHELL_EVENT);
    } else {
        hooks.insert(SHELL_EVENT.to_string(), Value::Array(kept));
    }
    config.to_string()
}

pub fn parse_hook_input(text: &str) -> Result<HookInput, String> {
    serde_json::from_str(text).map_err(|e| format!("unreadable cursor hook input: {e}"))
}
