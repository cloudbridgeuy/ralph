use super::{shell_quote, BlockedContent, PermissionOption, ALLOW_ONCE};
use crate::chat_socket::{runtime_dir_name, StoreKey};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

const SHELL_EVENT: &str = "beforeShellExecution";
const BRIDGE_ARGS: &str = " hook-bridge --socket ";
const DENY: &str = "deny";
pub const BRIDGE_TIMEOUT: Duration = Duration::from_secs(120);
const CURSOR_HOOK_TIMEOUT_SECS: u64 = 150;

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
    entries.push(json!({
        "command": command,
        "timeout": CURSOR_HOOK_TIMEOUT_SECS,
        "failClosed": true,
    }));
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookOutcome {
    Answered(String),
    TimedOut,
    BridgeGone(String),
    BadInput(String),
}

pub fn hook_options() -> Vec<PermissionOption> {
    let option = |id: &str, name: &str, kind: &str| PermissionOption {
        option_id: id.to_string(),
        name: name.to_string(),
        kind: kind.to_string(),
    };
    vec![
        option(ALLOW_ONCE, "Allow once", "allow_once"),
        option(DENY, "Deny", "reject_once"),
    ]
}

fn deny_reason(outcome: &HookOutcome) -> Option<String> {
    match outcome {
        HookOutcome::Answered(option) if option == ALLOW_ONCE => None,
        HookOutcome::Answered(_) => Some("the person denied it".to_string()),
        HookOutcome::TimedOut => Some(format!("no answer within {} s", BRIDGE_TIMEOUT.as_secs())),
        HookOutcome::BridgeGone(error) => {
            Some(format!("the ralph permission bridge failed: {error}"))
        }
        HookOutcome::BadInput(error) => Some(format!("unreadable hook input: {error}")),
    }
}

pub fn hook_output(outcome: &HookOutcome) -> String {
    match deny_reason(outcome) {
        None => json!({ "permission": "allow" }),
        Some(reason) => json!({
            "permission": DENY,
            "user_message": format!("ralph denied this shell command: {reason}."),
            "agent_message": format!(
                "ralph denied this shell command: {reason}. Do not retry it; say in your reply what you needed it for."
            ),
        }),
    }
    .to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeAsk {
    pub tool_use_id: String,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeAnswer {
    pub option_id: String,
}

pub fn hook_request_id(tool_use_id: &str, counter: u64) -> String {
    match tool_use_id.trim() {
        "" => format!("hook-{counter}"),
        id => id.to_string(),
    }
}

pub fn read_outcome(read: io::Result<String>) -> HookOutcome {
    match read {
        Ok(line) if line.trim().is_empty() => {
            HookOutcome::BridgeGone("the worker closed the bridge without an answer".to_string())
        }
        Ok(line) => serde_json::from_str::<BridgeAnswer>(line.trim()).map_or_else(
            |e| HookOutcome::BridgeGone(format!("unreadable bridge answer: {e}")),
            |answer| HookOutcome::Answered(answer.option_id),
        ),
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            HookOutcome::TimedOut
        }
        Err(e) => HookOutcome::BridgeGone(e.to_string()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeWait {
    Wait,
    GiveUp,
}

pub fn bridge_wait(elapsed: Duration, hook_connected: bool) -> BridgeWait {
    if hook_connected && elapsed < BRIDGE_TIMEOUT {
        BridgeWait::Wait
    } else {
        BridgeWait::GiveUp
    }
}

pub fn hook_content(request_id: &str, ask: &BridgeAsk) -> BlockedContent {
    BlockedContent {
        request_id: request_id.to_string(),
        tool: "shell".to_string(),
        summary: ask.command.clone(),
        options: hook_options(),
        pending: Vec::new(),
    }
}

pub fn hook_socket_name(key: &StoreKey, pid: u32, serial: u64) -> String {
    format!("{}-hook-{serial}.sock", runtime_dir_name(key, pid))
}
