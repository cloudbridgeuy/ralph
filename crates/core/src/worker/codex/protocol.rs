use crate::worker::{Selection, SessionRecord, SERVER_NAME};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;

pub const INITIALIZE: &str = "initialize";
pub const INITIALIZED: &str = "initialized";
pub const ACCOUNT_READ: &str = "account/read";
pub const MODEL_LIST: &str = "model/list";
pub const THREAD_START: &str = "thread/start";
pub const THREAD_RESUME: &str = "thread/resume";
pub const TURN_START: &str = "turn/start";
pub const TURN_INTERRUPT: &str = "turn/interrupt";
pub const MCP_STARTUP_STATUS: &str = "mcpServer/startupStatus/updated";
pub const PROBED_VERSION: &str = "0.164.0";
pub const NOT_LOGGED_IN: &str = "codex is not logged in; run `codex login`";
const MIN_VERSION: (u64, u64) = (0, 162);

pub fn initialize_params(version: &str) -> Value {
    json!({
        "clientInfo": {"name": "ralph", "version": version},
        "capabilities": {"experimentalApi": true}
    })
}

pub const MODEL_LIST_MAX_PAGES: usize = 20;

pub fn model_list_params(cursor: Option<&str>) -> Value {
    let mut params = json!({"limit": 200, "includeHidden": true});
    if let Some(cursor) = cursor {
        params["cursor"] = json!(cursor);
    }
    params
}

pub fn next_cursor(page: &Value) -> Option<String> {
    page["nextCursor"].as_str().map(str::to_string)
}

pub fn thread_start_params(cwd: &Path, selection: &Selection, mcp_server: &Value) -> Value {
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
    if let Some(model) = &selection.model {
        params["model"] = json!(model);
    }
    if let Some(effort) = &selection.effort {
        params["config"]["model_reasoning_effort"] = json!(effort);
    }
    params
}

pub fn record_selection(record: &SessionRecord) -> Selection {
    Selection {
        model: record.model.clone(),
        effort: record.effort.clone(),
    }
}

pub fn thread_resume_params(record: &SessionRecord, mcp_server: &Value) -> Value {
    let mut params = thread_start_params(&record.cwd, &record_selection(record), mcp_server);
    params["threadId"] = json!(record.session_id);
    params["excludeTurns"] = json!(true);
    params
}

pub fn turn_start_params(thread_id: &str, text: &str, selection: &Selection) -> Value {
    let mut params = json!({"threadId": thread_id, "input": [{"type": "text", "text": text}]});
    if let Some(model) = &selection.model {
        params["model"] = json!(model);
    }
    if let Some(effort) = &selection.effort {
        params["effort"] = json!(effort);
    }
    params
}

pub fn turn_interrupt_params(thread_id: &str, turn_id: &str) -> Value {
    json!({"threadId": thread_id, "turnId": turn_id})
}

pub fn decode_turn_id(started: &Value) -> Option<String> {
    started["turn"]["id"].as_str().map(str::to_string)
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
    let probed = parse_version(PROBED_VERSION).unwrap_or(MIN_VERSION);
    match (parsed.cmp(&MIN_VERSION), parsed.cmp(&probed)) {
        (std::cmp::Ordering::Less, _) => Err(format!(
            "codex {found} is too old; needs codex-cli >= {}.{} (probed {PROBED_VERSION})",
            MIN_VERSION.0, MIN_VERSION.1
        )),
        (_, std::cmp::Ordering::Greater) => Ok(Some(format!(
            "codex {found} is newer than the probed {PROBED_VERSION}; the app-server protocol may differ."
        ))),
        _ => Ok(None),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadInfo {
    pub id: String,
    pub model: Option<String>,
    pub effort: Option<String>,
}

pub fn decode_thread(started: &Value) -> Option<ThreadInfo> {
    let text = |value: &Value| value.as_str().map(str::to_string);
    Some(ThreadInfo {
        id: text(&started["thread"]["id"])?,
        model: text(&started["model"]),
        effort: text(&started["reasoningEffort"]),
    })
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
        assert_eq!(check_version("0.164.0"), Ok(None));
        assert_eq!(check_version("0.163.0"), Ok(None));
        let warning = check_version("0.165.0").unwrap().unwrap();
        assert!(warning.contains("0.165.0") && warning.contains("0.164.0"));
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
        let params = thread_start_params(
            Path::new("/w"),
            &Selection {
                model: Some("gpt-6.1-sol".into()),
                effort: None,
            },
            &server,
        );
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
        assert!(
            thread_start_params(Path::new("/w"), &Selection::default(), &server)
                .get("model")
                .is_none()
        );
        let effort = Selection {
            model: None,
            effort: Some("high".into()),
        };
        let params = thread_start_params(Path::new("/w"), &effort, &server);
        assert_eq!(params["config"]["model_reasoning_effort"], "high");
        assert!(params["config"]["mcp_servers"].is_object());
    }

    #[test]
    fn thread_resume_reuses_start_params_for_the_saved_thread_without_history() {
        let server = json!({"name": "ralph-worker", "command": "/bin/ralph", "args": ["worker-mcp"], "env": []});
        let record = SessionRecord {
            session_id: "thread-1".into(),
            cwd: "/w".into(),
            inbox: "/i".into(),
            store: Some("/s".into()),
            model: Some("m".into()),
            effort: Some("low".into()),
            permissions: None,
        };
        let params = thread_resume_params(&record, &server);
        let mut expected =
            thread_start_params(Path::new("/w"), &record_selection(&record), &server);
        expected["threadId"] = json!("thread-1");
        expected["excludeTurns"] = json!(true);
        assert_eq!(params, expected);
        assert_eq!(params["config"]["model_reasoning_effort"], "low");
        assert_eq!(
            params["config"]["mcp_servers"]["ralph-worker"]["required"],
            true
        );
        assert_eq!(params["model"], "m");
    }

    #[test]
    fn turn_start_carries_model_and_effort_only_when_selected() {
        let bare = turn_start_params("t", "hi", &Selection::default());
        assert_eq!(
            bare,
            json!({"threadId": "t", "input": [{"type": "text", "text": "hi"}]})
        );
        let chosen = Selection {
            model: Some("m".into()),
            effort: Some("low".into()),
        };
        let params = turn_start_params("t", "hi", &chosen);
        assert_eq!(
            (params["model"].as_str(), params["effort"].as_str()),
            (Some("m"), Some("low"))
        );
    }

    #[test]
    fn model_list_pages_follow_next_cursor() {
        assert!(model_list_params(None).get("cursor").is_none());
        assert_eq!(model_list_params(Some("c2"))["cursor"], "c2");
        assert_eq!(
            next_cursor(&json!({"data": [], "nextCursor": "c2"})).as_deref(),
            Some("c2")
        );
        assert_eq!(next_cursor(&json!({"data": [], "nextCursor": null})), None);
    }

    #[test]
    fn thread_info_is_read_from_the_thread_start_result() {
        let started = frame(
            r#"{"thread":{"id":"01a1655f-e1b3-7871-93ff-c4273c3bac74","cwd":"/w","status":{"type":"idle"}},"model":"m","reasoningEffort":"high","approvalPolicy":"on-request"}"#,
        );
        assert_eq!(
            decode_thread(&started),
            Some(ThreadInfo {
                id: "01a1655f-e1b3-7871-93ff-c4273c3bac74".into(),
                model: Some("m".into()),
                effort: Some("high".into())
            })
        );
        let bare = decode_thread(&json!({"thread": {"id": "x"}})).unwrap();
        assert_eq!((bare.model, bare.effort), (None, None));
        assert_eq!(decode_thread(&json!({})), None);
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
