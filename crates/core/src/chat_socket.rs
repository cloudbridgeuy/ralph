use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const SOCKET_TOOLS: [&str; 5] = [
    "spawn_worker",
    "reply",
    "close_worker",
    "watch",
    "suggest_reply",
];
const SOCKET_PREFIX: &str = "chat-";
const SOCKET_FILE: &str = "chat.sock";
const MCP_CONFIG_FILE: &str = "mcp.json";
const SIDECAR_SUFFIX: &str = ".owner";
pub const MAX_SOCKET_PATH_BYTES: usize = 103;

pub fn is_socket_tool(name: &str) -> bool {
    SOCKET_TOOLS.contains(&name)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SocketRequest {
    pub tool: String,
    pub args: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SocketResponse {
    pub text: String,
    pub is_error: bool,
}

pub fn encode_line<T: Serialize>(message: &T) -> String {
    format!("{}\n", json!(message))
}

pub fn decode_line<T: DeserializeOwned>(line: &str) -> Result<T, String> {
    serde_json::from_str(line.trim()).map_err(|e| format!("Malformed chat socket message: {e}"))
}

pub fn dir_name(pid: u32) -> String {
    format!("{SOCKET_PREFIX}{pid}")
}

pub fn socket_file_name() -> &'static str {
    SOCKET_FILE
}

pub fn mcp_config_path(socket: &Path) -> PathBuf {
    socket.with_file_name(MCP_CONFIG_FILE)
}

pub fn sidecar_name(pid: u32) -> String {
    format!("{SOCKET_PREFIX}{pid}{SIDECAR_SUFFIX}")
}

pub fn dir_pid(file_name: &str) -> Option<u32> {
    file_name.strip_prefix(SOCKET_PREFIX)?.parse().ok()
}

pub fn sidecar_pid(file_name: &str) -> Option<u32> {
    file_name
        .strip_prefix(SOCKET_PREFIX)?
        .strip_suffix(SIDECAR_SUFFIX)?
        .parse()
        .ok()
}

pub fn check_socket_path(path: &Path) -> Result<(), String> {
    let len = path.as_os_str().len();
    if len > MAX_SOCKET_PATH_BYTES {
        return Err(format!(
            "Chat socket path is {len} bytes, over the {MAX_SOCKET_PATH_BYTES} byte Unix socket limit: {}. Use a shorter --store path.",
            path.display()
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SocketOwner {
    pub pid: u32,
    pub start: String,
}

impl SocketOwner {
    pub fn parse(text: &str) -> Option<Self> {
        serde_json::from_str(text).ok()
    }

    pub fn render(&self) -> String {
        json!(self).to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SocketFacts<'a> {
    pub file_pid: u32,
    pub owner: Option<&'a SocketOwner>,
    pub owner_start_now: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketFate {
    Keep,
    Delete,
}

pub fn socket_fate(facts: SocketFacts) -> SocketFate {
    match (facts.owner, facts.owner_start_now) {
        (Some(owner), Some(now)) if owner.pid == facts.file_pid && owner.start == now => {
            SocketFate::Keep
        }
        _ => SocketFate::Delete,
    }
}

pub fn orphan_sidecar_fate(facts: SocketFacts) -> SocketFate {
    socket_fate(facts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn mcp_config_lives_next_to_each_chats_socket() {
        let one = mcp_config_path(Path::new("/s/chat-1/chat.sock"));
        let two = mcp_config_path(Path::new("/s/chat-2/chat.sock"));
        assert_eq!(one, PathBuf::from("/s/chat-1/mcp.json"));
        assert_ne!(one, two);
        assert_ne!(one, PathBuf::from("/s/mcp.json"));
    }

    fn owner(pid: u32, start: &str) -> SocketOwner {
        SocketOwner {
            pid,
            start: start.to_string(),
        }
    }

    #[test]
    fn socket_tools_route_to_the_socket_and_memory_tools_do_not() {
        for name in [
            "spawn_worker",
            "reply",
            "close_worker",
            "watch",
            "suggest_reply",
        ] {
            assert!(is_socket_tool(name));
        }
        for name in ["zoom", "recall", "", "Reply"] {
            assert!(!is_socket_tool(name));
        }
    }

    #[test]
    fn request_and_response_round_trip_as_one_line() {
        let request = SocketRequest {
            tool: "reply".to_string(),
            args: json!({"id": "A", "text": "multi\nline"}),
        };
        let line = encode_line(&request);
        assert_eq!(line.matches('\n').count(), 1);
        assert!(line.ends_with('\n'));
        assert_eq!(decode_line::<SocketRequest>(&line), Ok(request));

        let response = SocketResponse {
            text: "done".to_string(),
            is_error: true,
        };
        assert_eq!(
            decode_line::<SocketResponse>(&encode_line(&response)),
            Ok(response)
        );
    }

    #[test]
    fn decode_rejects_malformed_lines() {
        assert!(decode_line::<SocketRequest>("not json")
            .unwrap_err()
            .contains("Malformed"));
        assert!(decode_line::<SocketResponse>("{}").is_err());
    }

    #[test]
    fn names_round_trip_the_pid() {
        assert_eq!(dir_name(42), "chat-42");
        assert_eq!(socket_file_name(), "chat.sock");
        assert_eq!(sidecar_name(42), "chat-42.owner");
        assert_eq!(dir_pid("chat-42"), Some(42));
        assert_eq!(dir_pid("chat-42.owner"), None);
        assert_eq!(dir_pid("chat-x"), None);
        assert_eq!(dir_pid("mcp.json"), None);
        assert_eq!(sidecar_pid("chat-42.owner"), Some(42));
        assert_eq!(sidecar_pid("chat-42"), None);
        assert_eq!(sidecar_pid("chat-x.owner"), None);
        assert_eq!(sidecar_pid("mcp.json"), None);
    }

    #[test]
    fn socket_path_limit_is_enforced() {
        assert!(check_socket_path(Path::new("/s/chat-1/chat.sock")).is_ok());
        let exact = PathBuf::from("a".repeat(MAX_SOCKET_PATH_BYTES));
        assert!(check_socket_path(&exact).is_ok());
        let long = PathBuf::from("a".repeat(MAX_SOCKET_PATH_BYTES + 1));
        let message = check_socket_path(&long).unwrap_err();
        assert!(message.contains("103"));
        assert!(message.contains("--store"));
    }

    #[test]
    fn owner_record_round_trips_and_rejects_junk() {
        let record = owner(7, "Mon Jan  1 00:00:00 2026");
        assert_eq!(SocketOwner::parse(&record.render()), Some(record));
        assert_eq!(SocketOwner::parse("junk"), None);
        assert_eq!(SocketOwner::parse(""), None);
    }

    #[test]
    fn socket_is_kept_only_when_owner_is_alive_with_matching_start() {
        let record = owner(7, "t1");
        let fate = |file_pid, owner: Option<&SocketOwner>, now| {
            socket_fate(SocketFacts {
                file_pid,
                owner,
                owner_start_now: now,
            })
        };
        assert_eq!(fate(7, Some(&record), Some("t1")), SocketFate::Keep);
        assert_eq!(fate(7, Some(&record), None), SocketFate::Delete);
        assert_eq!(fate(7, Some(&record), Some("t2")), SocketFate::Delete);
        assert_eq!(fate(7, None, Some("t1")), SocketFate::Delete);
        assert_eq!(fate(7, None, None), SocketFate::Delete);
        assert_eq!(fate(8, Some(&record), Some("t1")), SocketFate::Delete);
    }

    #[test]
    fn orphan_sidecar_is_kept_only_when_owner_is_alive_with_matching_start() {
        let record = owner(7, "t1");
        let fate = |file_pid, owner: Option<&SocketOwner>, now| {
            orphan_sidecar_fate(SocketFacts {
                file_pid,
                owner,
                owner_start_now: now,
            })
        };
        assert_eq!(fate(7, Some(&record), Some("t1")), SocketFate::Keep);
        assert_eq!(fate(7, Some(&record), None), SocketFate::Delete);
        assert_eq!(fate(7, Some(&record), Some("t2")), SocketFate::Delete);
        assert_eq!(fate(7, None, None), SocketFate::Delete);
        assert_eq!(fate(7, None, Some("t1")), SocketFate::Delete);
        assert_eq!(fate(8, Some(&record), Some("t1")), SocketFate::Delete);
    }
}
