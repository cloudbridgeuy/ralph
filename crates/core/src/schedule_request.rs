use crate::mcp::string_arg;
use crate::schedule::{parse_after, parse_at, Task, TaskKind, TaskStatus};
use chrono::{DateTime, Duration as ChronoDuration, FixedOffset, Utc};
use serde_json::{json, Value};
use std::time::Duration;

pub const SCHEDULE_DIR: &str = "schedule";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum When {
    At(String),
    After(Duration),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleRequest {
    pub id: String,
    pub command: String,
    pub follow_up: Option<String>,
    pub when: When,
    pub timeout_secs: Option<u64>,
}

impl ScheduleRequest {
    pub fn into_task(self, now: DateTime<Utc>, offset: FixedOffset) -> Result<Task, String> {
        let at = match &self.when {
            When::At(raw) => parse_at(raw, now, offset)?,
            When::After(delay) => ChronoDuration::from_std(*delay)
                .ok()
                .and_then(|delay| now.checked_add_signed(delay))
                .ok_or_else(|| "after is too large.".to_string())?,
        };
        Ok(Task {
            id: self.id,
            kind: TaskKind::Command,
            command: self.command,
            follow_up: self.follow_up,
            at,
            timeout_secs: self.timeout_secs,
            status: TaskStatus::Proposed,
            late: false,
        })
    }
}

pub fn schedule_definition() -> Value {
    json!({
        "name": "schedule",
        "description": "Propose a one-shot shell command to run later. This tool only proposes: it records the task as awaiting approval and arms nothing. The chat itself asks the person to approve the exact command and time before anything is armed. Before calling it, tell the person the command and when it should run. When an approved task runs, its report reaches the inbox tagged schedule:<id>. Give exactly one of at or after.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "Short name for the task, without spaces or slashes."},
                "command": {"type": "string", "description": "The exact shell command to run, as the person will approve it."},
                "follow_up": {"type": "string", "description": "What to do when the command ends."},
                "at": {"type": "string", "description": "When to run: an RFC 3339 timestamp, or a local time HH:MM meaning its next occurrence."},
                "after": {"type": "string", "description": "How long to wait before running, such as 90s, 10m, 2h or 1h30m."},
                "timeout_secs": {"type": "integer", "minimum": 1, "description": "Kill the command after this many seconds. Default: the configured watch timeout, 1 hour unless changed."}
            },
            "required": ["id", "command"]
        }
    })
}

pub fn parse_schedule_args(args: &Value) -> Result<ScheduleRequest, String> {
    let id = string_arg(args, "id")?;
    if id.chars().any(|c| c == '/' || c.is_whitespace()) {
        return Err(format!(
            "Schedule id '{id}' must not contain spaces or slashes."
        ));
    }
    Ok(ScheduleRequest {
        id,
        command: string_arg(args, "command")?,
        follow_up: optional_string(args, "follow_up")?,
        when: when_arg(args)?,
        timeout_secs: timeout_arg(args)?,
    })
}

fn optional_string(args: &Value, key: &str) -> Result<Option<String>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => string_arg(args, key).map(Some),
    }
}

fn when_arg(args: &Value) -> Result<When, String> {
    match (
        optional_string(args, "at")?,
        optional_string(args, "after")?,
    ) {
        (Some(at), None) => Ok(When::At(at)),
        (None, Some(after)) => parse_after(&after).map(When::After),
        _ => Err("Give exactly one of at or after.".to_string()),
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

pub fn proposed_message(id: &str) -> String {
    format!("schedule `{id}` proposed; the chat will ask the person to approve it")
}

pub fn duplicate_message(id: &str) -> String {
    format!("schedule `{id}` already exists")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-01-01T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn offset() -> FixedOffset {
        FixedOffset::east_opt(2 * 3600).unwrap()
    }

    fn request(extra: Value) -> Result<ScheduleRequest, String> {
        let mut args = json!({"id": "nightly", "command": "make test"});
        for (key, value) in extra.as_object().unwrap() {
            args[key] = value.clone();
        }
        parse_schedule_args(&args)
    }

    #[test]
    fn after_request_parses_with_defaults() {
        assert_eq!(
            request(json!({"after": "10m"})),
            Ok(ScheduleRequest {
                id: "nightly".to_string(),
                command: "make test".to_string(),
                follow_up: None,
                when: When::After(Duration::from_secs(600)),
                timeout_secs: None,
            })
        );
    }

    #[test]
    fn at_request_parses_optional_arguments() {
        assert_eq!(
            request(json!({"at": "14:30", "follow_up": "report", "timeout_secs": 5})),
            Ok(ScheduleRequest {
                id: "nightly".to_string(),
                command: "make test".to_string(),
                follow_up: Some("report".to_string()),
                when: When::At("14:30".to_string()),
                timeout_secs: Some(5),
            })
        );
    }

    #[test]
    fn exactly_one_of_at_and_after_is_required() {
        assert!(request(json!({})).is_err());
        assert!(request(json!({"at": "14:30", "after": "5m"})).is_err());
        assert!(request(json!({"at": null, "after": null})).is_err());
    }

    #[test]
    fn bad_values_are_rejected() {
        assert!(request(json!({"after": "0s"})).is_err());
        assert!(request(json!({"after": "soon"})).is_err());
        assert!(request(json!({"after": "5m", "timeout_secs": 0})).is_err());
        assert!(request(json!({"after": "5m", "timeout_secs": "9"})).is_err());
        assert!(request(json!({"after": "5m", "follow_up": ""})).is_err());
        assert!(request(json!({"after": "5m", "id": "a b"})).is_err());
        assert!(request(json!({"after": "5m", "id": "a/b"})).is_err());
        assert!(request(json!({"after": "5m", "command": " "})).is_err());
    }

    #[test]
    fn after_resolves_from_now_into_a_proposed_task() {
        let task = request(json!({"after": "1h30m", "timeout_secs": 7}))
            .unwrap()
            .into_task(now(), offset())
            .unwrap();
        assert_eq!(task.id, "nightly");
        assert_eq!(task.kind, TaskKind::Command);
        assert_eq!(task.at, now() + ChronoDuration::minutes(90));
        assert_eq!(task.status, TaskStatus::Proposed);
        assert_eq!(task.timeout_secs, Some(7));
        assert!(!task.late);
    }

    #[test]
    fn at_resolves_in_the_local_offset() {
        let task = request(json!({"at": "14:30"}))
            .unwrap()
            .into_task(now(), offset())
            .unwrap();
        assert_eq!(task.at, now() + ChronoDuration::minutes(150));
    }

    #[test]
    fn an_unreadable_at_fails_when_resolved() {
        assert!(request(json!({"at": "later"}))
            .unwrap()
            .into_task(now(), offset())
            .is_err());
    }

    #[test]
    fn an_overflowing_after_fails_when_resolved() {
        let huge = ScheduleRequest {
            id: "a".to_string(),
            command: "x".to_string(),
            follow_up: None,
            when: When::After(Duration::from_secs(u64::MAX)),
            timeout_secs: None,
        };
        assert!(huge.into_task(now(), offset()).is_err());
    }

    #[test]
    fn the_definition_requires_id_and_command_and_says_it_only_proposes() {
        let definition = schedule_definition();
        assert_eq!(definition["name"], "schedule");
        assert_eq!(
            definition["inputSchema"]["required"],
            json!(["id", "command"])
        );
        let description = definition["description"].as_str().unwrap();
        assert!(description.contains("only proposes"));
        assert!(description.contains("asks the person to approve"));
        for key in ["follow_up", "at", "after", "timeout_secs"] {
            assert!(definition["inputSchema"]["properties"][key].is_object());
        }
    }

    #[test]
    fn messages_name_the_task() {
        assert!(proposed_message("a").contains("`a`"));
        assert!(proposed_message("a").contains("approve"));
        assert!(duplicate_message("a").contains("`a`"));
    }
}
