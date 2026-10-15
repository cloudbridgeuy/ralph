use crate::mcp::string_arg;
use crate::watch::{WatchKind, WatchSpec, WATCH_STATUS};
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
                "timeout_secs": {"type": "integer", "minimum": 1, "description": "Kill the command after this many seconds. Default none."}
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
        id,
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
            id: "ci".to_string(),
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
