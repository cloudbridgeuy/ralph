use crate::worker::SERVER_NAME;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;

pub const INITIALIZE: &str = "initialize";
pub const INITIALIZED: &str = "initialized";
pub const ACCOUNT_READ: &str = "account/read";
pub const MODEL_LIST: &str = "model/list";
pub const THREAD_START: &str = "thread/start";
pub const TURN_START: &str = "turn/start";
pub const MCP_STARTUP_STATUS: &str = "mcpServer/startupStatus/updated";
pub const PROBED_VERSION: &str = "0.162.1";
pub const NOT_LOGGED_IN: &str = "codex is not logged in; run `codex login`";
const MIN_VERSION: (u64, u64) = (0, 162);

pub fn initialize_params(version: &str) -> Value {
    json!({
        "clientInfo": {"name": "ralph", "version": version},
        "capabilities": {"experimentalApi": true}
    })
}

pub fn model_list_params() -> Value {
    json!({"limit": 200, "includeHidden": true})
}

pub fn thread_start_params(cwd: &Path, model: Option<&str>, mcp_server: &Value) -> Value {
    let mut params = json!({
        "cwd": cwd,
        "approvalPolicy": "on-request",
        "approvalsReviewer": "user",
        "sandbox": "workspace-write",
        "config": {
            "mcp_servers": {
                (SERVER_NAME): {
                    "command": mcp_server["command"],
                    "args": mcp_server["args"],
                    "required": true,
                    "default_tools_approval_mode": "approve"
                }
            }
        }
    });
    if let Some(model) = model {
        params["model"] = json!(model);
    }
    params
}

pub fn turn_start_params(thread_id: &str, text: &str) -> Value {
    json!({"threadId": thread_id, "input": [{"type": "text", "text": text}]})
}

pub fn decode_version(initialize: &Value) -> Option<String> {
    let agent = initialize.get("userAgent")?.as_str()?;
    let (_, version) = agent.split_whitespace().next()?.rsplit_once('/')?;
    Some(version.to_string())
}

fn parse_version(version: &str) -> Option<(u64, u64)> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    parts
        .next()?
        .split('-')
        .next()?
        .parse::<u64>()
        .ok()
        .map(|_| (major, minor))
}

pub fn check_version(found: &str) -> Result<Option<String>, String> {
    let parsed = parse_version(found)
        .ok_or_else(|| format!("could not read the codex version from `{found}`"))?;
    match parsed.cmp(&MIN_VERSION) {
        std::cmp::Ordering::Less => Err(format!(
            "codex {found} is too old; needs codex-cli >= {}.{} (probed {PROBED_VERSION})",
            MIN_VERSION.0, MIN_VERSION.1
        )),
        std::cmp::Ordering::Equal => Ok(None),
        std::cmp::Ordering::Greater => Ok(Some(format!(
            "codex {found} is newer than the probed {PROBED_VERSION}; the app-server protocol may differ."
        ))),
    }
}

pub fn check_login(account: &Value) -> Result<(), String> {
    let logged_out = account["account"].is_null() && account["requiresOpenaiAuth"] == json!(true);
    if logged_out {
        Err(NOT_LOGGED_IN.to_string())
    } else {
        Ok(())
    }
}

pub fn model_ids(list: &Value) -> Vec<String> {
    list["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|model| model["id"].as_str().or_else(|| model["model"].as_str()))
        .map(str::to_string)
        .collect()
}

pub fn check_model(requested: Option<&str>, available: &[String]) -> Result<(), String> {
    match requested {
        Some(model) if !available.iter().any(|id| id == model) => Err(format!(
            "codex does not list model '{model}'; available: {}",
            available.join(", ")
        )),
        _ => Ok(()),
    }
}

pub fn decode_thread_id(started: &Value) -> Option<String> {
    started["thread"]["id"].as_str().map(str::to_string)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum McpState {
    Starting,
    Ready,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpVerdict {
    Waiting,
    Ready,
    Failed { server: String, detail: String },
}

#[derive(Debug, Default)]
pub struct McpStartup {
    states: BTreeMap<String, McpState>,
}

impl McpStartup {
    pub fn observe(&mut self, method: &str, params: &Value) {
        let (true, Some(name), Some(status)) = (
            method == MCP_STARTUP_STATUS,
            params["name"].as_str(),
            params["status"].as_str(),
        ) else {
            return;
        };
        let state = match status {
            "ready" => McpState::Ready,
            "starting" => McpState::Starting,
            other => McpState::Failed(
                params["error"]
                    .as_str()
                    .map_or_else(|| other.to_string(), |error| format!("{other}: {error}")),
            ),
        };
        self.states.insert(name.to_string(), state);
    }

    pub fn verdict(&self, required: &[&str]) -> McpVerdict {
        let mut waiting = false;
        for server in required {
            match self.states.get(*server) {
                Some(McpState::Failed(detail)) => {
                    return McpVerdict::Failed {
                        server: (*server).to_string(),
                        detail: detail.clone(),
                    }
                }
                Some(McpState::Ready) => {}
                Some(McpState::Starting) | None => waiting = true,
            }
        }
        if waiting {
            McpVerdict::Waiting
        } else {
            McpVerdict::Ready
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(line: &str) -> Value {
        serde_json::from_str(line).unwrap()
    }

    #[test]
    fn version_comes_from_the_initialize_user_agent() {
        let init = frame(
            r#"{"userAgent":"ralph-probe/0.162.1 (Mac OS 26.3.0; arm64) tmux-256color (ralph-probe; 0)","codexHome":"/h","platformFamily":"unix","platformOs":"macos"}"#,
        );
        assert_eq!(decode_version(&init).as_deref(), Some("0.162.1"));
        assert_eq!(decode_version(&json!({})), None);
    }

    #[test]
    fn versions_older_than_the_probe_are_refused_and_newer_ones_warn() {
        assert_eq!(check_version("0.162.1"), Ok(None));
        assert_eq!(check_version("0.162.0"), Ok(None));
        assert_eq!(check_version("0.162.9-alpha.1"), Ok(None));
        let old = check_version("0.161.4").unwrap_err();
        assert!(old.contains("0.161.4") && old.contains(">= 0.162"));
        let warning = check_version("0.163.0").unwrap().unwrap();
        assert!(warning.contains("0.163.0") && warning.contains("0.162.1"));
        assert!(check_version("1.0.0").unwrap().is_some());
        assert!(check_version("garbage").is_err());
    }

    #[test]
    fn a_missing_account_that_needs_auth_is_not_logged_in() {
        let out = frame(r#"{"account":null,"requiresOpenaiAuth":true,"workspaceRouting":null}"#);
        assert_eq!(check_login(&out), Err(NOT_LOGGED_IN.to_string()));
        let chatgpt = frame(
            r#"{"account":{"type":"chatgpt","email":"a@b.c","planType":"prolite"},"requiresOpenaiAuth":true}"#,
        );
        assert_eq!(check_login(&chatgpt), Ok(()));
    }

    #[test]
    fn models_are_listed_by_id_and_unknown_ones_name_the_choices() {
        let list = frame(
            r#"{"data":[{"id":"gpt-6.1-sol","model":"gpt-6.1-sol","isDefault":true},{"id":"gpt-6-astra","model":"gpt-6-astra","isDefault":false}],"nextCursor":null}"#,
        );
        let ids = model_ids(&list);
        assert_eq!(ids, ["gpt-6.1-sol", "gpt-6-astra"]);
        assert_eq!(check_model(None, &ids), Ok(()));
        assert_eq!(check_model(Some("gpt-6-astra"), &ids), Ok(()));
        let error = check_model(Some("nope"), &ids).unwrap_err();
        assert!(error.contains("nope") && error.contains("gpt-6.1-sol, gpt-6-astra"));
    }

    #[test]
    fn thread_start_attaches_the_report_server_with_approve_mode() {
        let server = json!({"name": "ralph-worker", "command": "/bin/ralph", "args": ["worker-mcp", "--id", "A"], "env": []});
        let params = thread_start_params(Path::new("/w"), Some("gpt-6.1-sol"), &server);
        assert_eq!(
            params,
            json!({
                "cwd": "/w",
                "model": "gpt-6.1-sol",
                "approvalPolicy": "on-request",
                "approvalsReviewer": "user",
                "sandbox": "workspace-write",
                "config": {"mcp_servers": {"ralph-worker": {
                    "command": "/bin/ralph",
                    "args": ["worker-mcp", "--id", "A"],
                    "required": true,
                    "default_tools_approval_mode": "approve"
                }}}
            })
        );
        assert!(thread_start_params(Path::new("/w"), None, &server)
            .get("model")
            .is_none());
    }

    #[test]
    fn thread_id_is_read_from_the_thread_start_result() {
        let started = frame(
            r#"{"thread":{"id":"01a1655f-e1b3-7871-93ff-c4273c3bac74","cwd":"/w","status":{"type":"idle"}},"model":"m","approvalPolicy":"on-request"}"#,
        );
        assert_eq!(
            decode_thread_id(&started).as_deref(),
            Some("01a1655f-e1b3-7871-93ff-c4273c3bac74")
        );
        assert_eq!(decode_thread_id(&json!({})), None);
    }

    #[test]
    fn startup_waits_for_the_required_server_and_fails_on_its_failure() {
        let mut startup = McpStartup::default();
        assert_eq!(startup.verdict(&["ralph-worker"]), McpVerdict::Waiting);
        for line in [
            r#"{"method":"mcpServer/startupStatus/updated","params":{"threadId":"t","name":"mcptools","status":"failed","error":"x","failureReason":null}}"#,
            r#"{"method":"mcpServer/startupStatus/updated","params":{"threadId":"t","name":"ralph-worker","status":"starting","error":null,"failureReason":null}}"#,
        ] {
            let frame = frame(line);
            startup.observe(frame["method"].as_str().unwrap(), &frame["params"]);
        }
        assert_eq!(startup.verdict(&["ralph-worker"]), McpVerdict::Waiting);
        startup.observe(
            MCP_STARTUP_STATUS,
            &json!({"name": "ralph-worker", "status": "ready", "error": null}),
        );
        assert_eq!(startup.verdict(&["ralph-worker"]), McpVerdict::Ready);
        startup.observe(
            MCP_STARTUP_STATUS,
            &json!({"name": "ralph-worker", "status": "failed", "error": "spawn failed"}),
        );
        assert_eq!(
            startup.verdict(&["ralph-worker"]),
            McpVerdict::Failed {
                server: "ralph-worker".into(),
                detail: "failed: spawn failed".into()
            }
        );
    }
}
