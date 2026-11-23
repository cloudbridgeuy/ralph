use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

const SOCKET_TOOLS: [&str; 10] = [
    "workers",
    "spawn_worker",
    "reply",
    "close_worker",
    "watch",
    "watch_list",
    "watch_stop",
    "schedule",
    "propose_run",
    "step_result",
];
const SOCKET_PREFIX: &str = "chat-";
const RUNTIME_PARENT: &str = ".ralph";
const RUNTIME_DIR: &str = "run";
const STORE_KEY_HEX_CHARS: usize = 12;
const SERVE_DIR_PREFIX: &str = "serve-";
const SERVE_SOCKET_FILE: &str = "serve.sock";
const SOCKET_SUFFIX: &str = ".sock";
const MCP_CONFIG_FILE: &str = "mcp.json";
const SYSTEM_PROMPT_FILE: &str = "system-prompt.md";
const SKILL_PLUGIN_DIR: &str = "plugin";
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

pub const MAX_FRAME_BYTES: usize = 1 << 20;

pub fn encode_line<T: Serialize>(message: &T) -> String {
    format!("{}\n", json!(message))
}

pub fn decode_line<T: DeserializeOwned>(line: &str) -> Result<T, String> {
    serde_json::from_str(line.trim()).map_err(|e| format!("Malformed chat socket message: {e}"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreKey(String);

impl StoreKey {
    pub fn of(canonical_store: &Path) -> Self {
        let digest = Sha256::digest(canonical_store.as_os_str().as_bytes());
        Self(
            digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()[..STORE_KEY_HEX_CHARS]
                .to_string(),
        )
    }
}

pub fn runtime_root(home: &Path) -> PathBuf {
    home.join(RUNTIME_PARENT).join(RUNTIME_DIR)
}

pub fn runtime_dir_name(key: &StoreKey, pid: u32) -> String {
    format!("{}-{pid}", key.0)
}

pub fn socket_file_name(key: &StoreKey, pid: u32) -> String {
    format!("{}{SOCKET_SUFFIX}", runtime_dir_name(key, pid))
}

pub fn socket_file_pid(key: &StoreKey, file_name: &str) -> Option<u32> {
    file_name
        .strip_prefix(&key.0)?
        .strip_prefix('-')?
        .strip_suffix(SOCKET_SUFFIX)?
        .parse()
        .ok()
}

pub fn serve_socket_path(root: &Path, key: &StoreKey) -> PathBuf {
    root.join(format!("{SERVE_DIR_PREFIX}{}", key.0))
        .join(SERVE_SOCKET_FILE)
}

pub fn chat_files_dir(store: &Path, pid: u32) -> PathBuf {
    store.join(format!("{SOCKET_PREFIX}{pid}"))
}

pub fn chat_files_dir_pid(file_name: &str) -> Option<u32> {
    file_name.strip_prefix(SOCKET_PREFIX)?.parse().ok()
}

pub fn mcp_config_path(files_dir: &Path) -> PathBuf {
    files_dir.join(MCP_CONFIG_FILE)
}

pub fn system_prompt_path(files_dir: &Path) -> PathBuf {
    files_dir.join(SYSTEM_PROMPT_FILE)
}

pub fn skill_plugin_dir(files_dir: &Path) -> PathBuf {
    files_dir.join(SKILL_PLUGIN_DIR)
}

pub fn sidecar_name(pid: u32) -> String {
    format!("{SOCKET_PREFIX}{pid}{SIDECAR_SUFFIX}")
}

pub fn sidecar_pid(file_name: &str) -> Option<u32> {
    file_name
        .strip_prefix(SOCKET_PREFIX)?
        .strip_suffix(SIDECAR_SUFFIX)?
        .parse()
        .ok()
}

pub fn check_socket_path(root: &Path, path: &Path) -> Result<(), String> {
    let len = path.as_os_str().len();
    if len > MAX_SOCKET_PATH_BYTES {
        return Err(format!(
            "Chat socket path is {len} bytes, over the {MAX_SOCKET_PATH_BYTES} byte Unix socket limit: {}. The runtime root {} is too long.",
            path.display(),
            root.display()
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeFacts {
    pub files_dir_exists: bool,
    pub socket_exists: bool,
    pub has_skills: bool,
    pub skill_plugin_exists: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryPlan {
    pub create_files_dir: bool,
    pub write_mcp_config: bool,
    pub write_skill_plugin: bool,
    pub rebind_socket: bool,
}

pub fn recovery_plan(facts: RuntimeFacts) -> RecoveryPlan {
    RecoveryPlan {
        create_files_dir: !facts.files_dir_exists,
        write_mcp_config: true,
        write_skill_plugin: facts.has_skills
            && (!facts.files_dir_exists || !facts.skill_plugin_exists),
        rebind_socket: !facts.socket_exists,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn mcp_config_lives_in_each_chats_store_files_dir() {
        let one = mcp_config_path(&chat_files_dir(Path::new("/s"), 1));
        let two = mcp_config_path(&chat_files_dir(Path::new("/s"), 2));
        assert_eq!(one, PathBuf::from("/s/chat-1/mcp.json"));
        assert_ne!(one, two);
        assert_ne!(one, PathBuf::from("/s/mcp.json"));
        assert!(!one.starts_with("/tmp"));
    }

    #[test]
    fn system_prompt_lives_in_each_chats_store_files_dir() {
        let dir = chat_files_dir(Path::new("/s"), 1);
        assert_eq!(
            system_prompt_path(&dir),
            PathBuf::from("/s/chat-1/system-prompt.md")
        );
        assert_eq!(skill_plugin_dir(&dir), PathBuf::from("/s/chat-1/plugin"));
    }

    #[test]
    fn the_serve_socket_lives_in_a_per_store_dir_apart_from_chat_sockets() {
        let key = StoreKey::of(Path::new("/s/store"));
        let root = runtime_root(Path::new("/Users/me"));
        let path = serve_socket_path(&root, &key);
        assert_eq!(
            path,
            PathBuf::from(format!("/Users/me/.ralph/run/serve-{}/serve.sock", key.0))
        );
        assert_eq!(path.parent().unwrap().file_name().unwrap().len(), 18);
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
            "workers",
            "spawn_worker",
            "reply",
            "close_worker",
            "watch",
            "watch_list",
            "watch_stop",
            "schedule",
            "propose_run",
            "step_result",
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
        let key = StoreKey::of(Path::new("/s"));
        let sock = socket_file_name(&key, 42);
        assert!(sock.ends_with("-42.sock"));
        assert_eq!(socket_file_pid(&key, &sock), Some(42));
        assert_eq!(socket_file_pid(&key, &runtime_dir_name(&key, 42)), None);
        assert_eq!(socket_file_pid(&key, "chat-42.sock"), None);
        assert_eq!(socket_file_pid(&key, "mcp.json"), None);
        assert_eq!(chat_files_dir_pid("chat-42"), Some(42));
        assert_eq!(chat_files_dir_pid("chat-42.owner"), None);
        assert_eq!(chat_files_dir_pid("chat-x"), None);
        assert_eq!(sidecar_name(42), "chat-42.owner");
        assert_eq!(sidecar_pid("chat-42.owner"), Some(42));
        assert_eq!(sidecar_pid("chat-42"), None);
        assert_eq!(sidecar_pid("chat-x.owner"), None);
        assert_eq!(sidecar_pid("mcp.json"), None);
    }

    #[test]
    fn runtime_root_lives_under_home_not_tmp() {
        let root = runtime_root(Path::new("/Users/alice"));
        assert_eq!(root, PathBuf::from("/Users/alice/.ralph/run"));
        assert!(!root.starts_with("/tmp"));
        assert_ne!(root, runtime_root(Path::new("/Users/bob")));
    }

    #[test]
    fn store_key_is_twelve_hex_chars_and_stable_per_store() {
        let key = StoreKey::of(Path::new("/s"));
        let name = runtime_dir_name(&key, 1);
        let (hex, pid) = name.split_once('-').unwrap();
        assert_eq!(pid, "1");
        assert_eq!(hex.len(), 12);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(key, StoreKey::of(Path::new("/s")));
        assert_ne!(key, StoreKey::of(Path::new("/s/threads/viewer")));
    }

    #[test]
    fn socket_file_differs_per_store_and_pid() {
        let main = StoreKey::of(Path::new("/s"));
        let thread = StoreKey::of(Path::new("/s/threads/viewer"));
        assert_ne!(socket_file_name(&main, 1), socket_file_name(&main, 2));
        assert_ne!(socket_file_name(&main, 1), socket_file_name(&thread, 1));
    }

    #[test]
    fn socket_file_pid_reads_only_this_stores_flat_sockets() {
        let main = StoreKey::of(Path::new("/s"));
        let thread = StoreKey::of(Path::new("/s/threads/viewer"));
        assert_eq!(socket_file_pid(&main, &socket_file_name(&thread, 7)), None);
        assert_eq!(socket_file_pid(&main, &socket_file_name(&main, 7)), Some(7));
        let bad = format!("{}x.sock", runtime_dir_name(&main, 7));
        assert_eq!(socket_file_pid(&main, &bad), None);
    }

    #[test]
    fn deep_thread_store_socket_fits_the_limit() {
        let store = Path::new(
            "/Users/guzmanmonne/Projects/Vntana/vntana-model-ops/branches/main/.claude/memory/threads/viewer",
        );
        let root = runtime_root(Path::new("/Users/guzmanmonne"));
        let socket = root.join(socket_file_name(&StoreKey::of(store), u32::MAX));
        assert!(socket.starts_with("/Users/guzmanmonne/.ralph/run"));
        assert!(socket.as_os_str().len() <= MAX_SOCKET_PATH_BYTES);
        assert_eq!(check_socket_path(&root, &socket), Ok(()));
    }

    #[test]
    fn socket_path_limit_is_enforced() {
        let root = Path::new("/Users/alice/.ralph/run");
        assert!(check_socket_path(root, Path::new("/s/chat-1.sock")).is_ok());
        let exact = PathBuf::from("a".repeat(MAX_SOCKET_PATH_BYTES));
        assert!(check_socket_path(root, &exact).is_ok());
        let long = PathBuf::from("a".repeat(MAX_SOCKET_PATH_BYTES + 1));
        let message = check_socket_path(root, &long).unwrap_err();
        assert!(message.contains("103"));
        assert!(message.contains("/Users/alice/.ralph/run"));
        assert!(!message.contains("--store"));
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

    #[test]
    fn recovery_plan_rewrites_mcp_every_turn_and_rebinds_socket_only_when_missing() {
        let intact = recovery_plan(RuntimeFacts {
            files_dir_exists: true,
            socket_exists: true,
            has_skills: false,
            skill_plugin_exists: false,
        });
        assert_eq!(
            intact,
            RecoveryPlan {
                create_files_dir: false,
                write_mcp_config: true,
                write_skill_plugin: false,
                rebind_socket: false,
            }
        );

        let dir_wiped_with_skills = recovery_plan(RuntimeFacts {
            files_dir_exists: false,
            socket_exists: true,
            has_skills: true,
            skill_plugin_exists: false,
        });
        assert_eq!(
            dir_wiped_with_skills,
            RecoveryPlan {
                create_files_dir: true,
                write_mcp_config: true,
                write_skill_plugin: true,
                rebind_socket: false,
            }
        );

        let socket_missing = recovery_plan(RuntimeFacts {
            files_dir_exists: true,
            socket_exists: false,
            has_skills: true,
            skill_plugin_exists: true,
        });
        assert_eq!(
            socket_missing,
            RecoveryPlan {
                create_files_dir: false,
                write_mcp_config: true,
                write_skill_plugin: false,
                rebind_socket: true,
            }
        );

        let both_missing = recovery_plan(RuntimeFacts {
            files_dir_exists: false,
            socket_exists: false,
            has_skills: false,
            skill_plugin_exists: false,
        });
        assert_eq!(
            both_missing,
            RecoveryPlan {
                create_files_dir: true,
                write_mcp_config: true,
                write_skill_plugin: false,
                rebind_socket: true,
            }
        );
    }
}
