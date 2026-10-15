use crate::mcp::string_arg;
use crate::watch::{watch_id, WatchKind, WatchSpec, WATCH_STATUS};
use crate::watch_store::file_name;
use crate::worker::Report;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

pub const WATCH_REQUESTS_DIR: &str = "watch-requests";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchRequest {
    pub id: String,
    pub command: String,
    pub follow_up: String,
    pub persistent: bool,
    pub timeout_secs: Option<u64>,
}

impl WatchRequest {
    pub fn into_spec(self) -> WatchSpec {
        WatchSpec {
            id: self.id,
            command: self.command,
            follow_up: Some(self.follow_up),
            kind: if self.persistent {
                WatchKind::Persistent
            } else {
                WatchKind::Normal
            },
            timeout: self.timeout_secs.map(Duration::from_secs),
            limit: self.timeout_secs.map(Duration::from_secs),
        }
    }

    pub fn file_name(&self) -> String {
        file_name(&self.id)
    }
}

pub fn watch_definition() -> Value {
    json!({
        "name": "watch",
        "description": "Start a background watch: ralph runs a shell command and, when it exits, delivers its output and your follow-up instruction to the inbox as a watch report. Before calling it, show the person the exact command and wait for their approval. This tool does not ask for confirmation.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "Short name for the watch, without spaces or slashes."},
                "command": {"type": "string", "description": "The exact shell command to run, as the person approved it."},
                "follow_up": {"type": "string", "description": "What to do when the command ends."},
                "persistent": {"type": "boolean", "description": "Keep the watch across chat restarts. Default false."},
                "timeout_secs": {"type": "integer", "minimum": 1, "description": "Kill the command after this many seconds. Default: the configured watch timeout, 1 hour unless changed."}
            },
            "required": ["id", "command", "follow_up"]
        }
    })
}

pub fn parse_watch_args(args: &Value) -> Result<WatchRequest, String> {
    let id = string_arg(args, "id")?;
    if id.chars().any(|c| c == '/' || c.is_whitespace()) {
        return Err(format!(
            "Watch id '{id}' must not contain spaces or slashes."
        ));
    }
    Ok(WatchRequest {
        id: watch_id(&id),
        command: string_arg(args, "command")?,
        follow_up: string_arg(args, "follow_up")?,
        persistent: bool_arg(args, "persistent")?,
        timeout_secs: timeout_arg(args)?,
    })
}

fn bool_arg(args: &Value, key: &str) -> Result<bool, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(format!("{key} must be true or false.")),
    }
}

fn timeout_arg(args: &Value) -> Result<Option<u64>, String> {
    match args.get("timeout_secs") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => match value.as_u64() {
            Some(secs) if secs > 0 => Ok(Some(secs)),
            _ => Err("timeout_secs must be a positive whole number.".to_string()),
        },
    }
}

pub fn queued_message(id: &str) -> String {
    format!("Watch `{id}` is queued. Its report will arrive in the inbox when the command ends.")
}

pub fn already_queued_message(id: &str) -> String {
    format!("watch `{id}` is already queued")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedPlan {
    pub start: Vec<WatchSpec>,
    pub rejected: Vec<Report>,
}

pub fn watch_in_use_message(id: &str) -> String {
    format!("a watch named `{id}` is already running")
}

pub fn plan_queued(requests: Vec<WatchRequest>, used: &[String], at: &str) -> QueuedPlan {
    let mut taken: Vec<String> = used.to_vec();
    let mut plan = QueuedPlan {
        start: Vec::new(),
        rejected: Vec::new(),
    };
    for request in requests {
        if taken.contains(&request.id) {
            plan.rejected.push(start_failure_report(
                &request.id,
                &watch_in_use_message(&request.id),
                at,
            ));
        } else {
            taken.push(request.id.clone());
            plan.start.push(request.into_spec());
        }
    }
    plan
}

pub fn start_failure_report(id: &str, error: &str, at: &str) -> Report {
    Report {
        worker: id.to_string(),
        at: at.to_string(),
        status: WATCH_STATUS.to_string(),
        text: format!("Watch `{id}` did not start: {error}."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> WatchRequest {
        WatchRequest {
            id: "watch:ci".to_string(),
            command: "gh run watch 1".to_string(),
            follow_up: "report the result".to_string(),
            persistent: false,
            timeout_secs: None,
        }
    }

    #[test]
    fn parses_required_arguments_with_defaults() {
        let args =
            json!({"id": "ci", "command": "gh run watch 1", "follow_up": "report the result"});
        assert_eq!(parse_watch_args(&args), Ok(request()));
    }

    fn queued(id: &str) -> WatchRequest {
        WatchRequest {
            id: id.to_string(),
            ..request()
        }
    }

    #[test]
    fn queued_requests_with_free_ids_all_start() {
        let plan = plan_queued(vec![queued("watch:a"), queued("watch:b")], &[], "t");
        let ids: Vec<&str> = plan.start.iter().map(|spec| spec.id.as_str()).collect();
        assert_eq!(ids, ["watch:a", "watch:b"]);
        assert!(plan.rejected.is_empty());
    }

    #[test]
    fn queued_requests_for_active_or_saved_ids_are_rejected_with_a_report() {
        let used = vec!["watch:a".to_string()];
        let plan = plan_queued(vec![queued("watch:a"), queued("watch:b")], &used, "t1");
        assert_eq!(plan.start.len(), 1);
        assert_eq!(plan.start[0].id, "watch:b");
        let [report] = plan.rejected.as_slice() else {
            panic!("{plan:?}");
        };
        assert_eq!(report.worker, "watch:a");
        assert_eq!(report.at, "t1");
        assert_eq!(report.status, WATCH_STATUS);
        assert!(report
            .text
            .contains("did not start: a watch named `watch:a` is already running."));
    }

    #[test]
    fn a_repeated_id_in_one_batch_starts_once() {
        let plan = plan_queued(vec![queued("watch:a"), queued("watch:a")], &[], "t");
        assert_eq!(plan.start.len(), 1);
        assert_eq!(plan.rejected.len(), 1);
    }

    #[test]
    fn queued_requests_get_the_spec_of_their_request() {
        let mut request = queued("watch:a");
        request.persistent = true;
        request.timeout_secs = Some(5);
        let plan = plan_queued(vec![request], &[], "t");
        assert_eq!(plan.start[0].kind, WatchKind::Persistent);
        assert_eq!(plan.start[0].timeout, Some(Duration::from_secs(5)));
    }

    #[test]
    fn the_duplicate_message_names_the_id() {
        assert_eq!(
            already_queued_message("watch:ci"),
            "watch `watch:ci` is already queued"
        );
    }

    #[test]
    fn the_request_id_is_prefixed_once() {
        for id in ["ci", "watch:ci"] {
            let args = json!({"id": id, "command": "c", "follow_up": "f"});
            assert_eq!(parse_watch_args(&args).unwrap().id, "watch:ci");
        }
    }

    #[test]
    fn parses_optional_arguments() {
        let args = json!({"id": "ci", "command": "c", "follow_up": "f", "persistent": true, "timeout_secs": 30});
        let parsed = parse_watch_args(&args).unwrap();
        assert!(parsed.persistent);
        assert_eq!(parsed.timeout_secs, Some(30));
    }

    #[test]
    fn rejects_missing_required_arguments() {
        for missing in ["id", "command", "follow_up"] {
            let mut args = json!({"id": "ci", "command": "c", "follow_up": "f"});
            args.as_object_mut().unwrap().remove(missing);
            assert!(parse_watch_args(&args).unwrap_err().contains(missing));
        }
    }

    #[test]
    fn rejects_ids_with_slashes_or_whitespace() {
        for id in ["a/b", "a b", "a\tb"] {
            let args = json!({"id": id, "command": "c", "follow_up": "f"});
            assert!(parse_watch_args(&args).is_err());
        }
    }

    #[test]
    fn rejects_bad_timeout_and_persistent_values() {
        for bad in [json!(0), json!(-1), json!(1.5), json!("5")] {
            let args = json!({"id": "a", "command": "c", "follow_up": "f", "timeout_secs": bad});
            assert!(parse_watch_args(&args)
                .unwrap_err()
                .contains("timeout_secs"));
        }
        let args = json!({"id": "a", "command": "c", "follow_up": "f", "persistent": "yes"});
        assert!(parse_watch_args(&args).unwrap_err().contains("persistent"));
    }

    #[test]
    fn into_spec_maps_kind_follow_up_and_timeout() {
        let spec = WatchRequest {
            persistent: true,
            timeout_secs: Some(9),
            ..request()
        }
        .into_spec();
        assert_eq!(spec.kind, WatchKind::Persistent);
        assert_eq!(spec.follow_up.as_deref(), Some("report the result"));
        assert_eq!(spec.timeout, Some(Duration::from_secs(9)));
        assert_eq!(request().into_spec().kind, WatchKind::Normal);
        assert_eq!(request().into_spec().timeout, None);
    }

    #[test]
    fn file_name_is_a_json_name_free_of_the_raw_id() {
        let name = request().file_name();
        assert!(name.ends_with(".json"));
        assert!(!name.contains("ci"));
    }

    #[test]
    fn failure_report_uses_the_watch_status() {
        let report = start_failure_report("ci", "a watch named `ci` is already running", "t");
        assert_eq!(report.status, WATCH_STATUS);
        assert_eq!(report.worker, "ci");
        assert!(report.text.contains("did not start"));
        assert!(report.text.contains("already running"));
    }

    #[test]
    fn definition_requires_id_command_and_follow_up() {
        assert_eq!(
            watch_definition()["inputSchema"]["required"],
            json!(["id", "command", "follow_up"])
        );
    }
}
