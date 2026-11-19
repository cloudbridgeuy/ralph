use crate::mcp::string_arg;
use crate::schedule::{
    escape_controls, parse_after, parse_at, Task, TaskKind, TaskStatus, WORKER_MAX_TIMEOUT_SECS,
};
use crate::schedule_repeat::{
    check_min, first_fire, next_fire, parse_repeat, repeat_label, Repeat,
};
use crate::worker::is_dashed_token;
use chrono::{DateTime, Duration as ChronoDuration, TimeZone, Utc};
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
    pub kind: TaskKind,
    pub follow_up: Option<String>,
    pub when: Option<When>,
    pub repeat: Option<Repeat>,
    pub timeout_secs: Option<u64>,
}

impl ScheduleRequest {
    pub fn into_task<Tz: TimeZone>(self, now: DateTime<Utc>, zone: &Tz) -> Result<Task, String> {
        let at = match &self.when {
            Some(When::At(raw)) => parse_at(raw, now, zone)?,
            Some(When::After(delay)) => ChronoDuration::from_std(*delay)
                .ok()
                .and_then(|delay| now.checked_add_signed(delay))
                .ok_or_else(|| "after is too large.".to_string())?,
            None => self
                .repeat
                .as_ref()
                .and_then(|repeat| first_fire(repeat, now, zone))
                .ok_or_else(|| {
                    "The first run of the repeating task cannot be computed.".to_string()
                })?,
        };
        if let Some(repeat) = &self.repeat {
            if next_fire(repeat, at, at, zone).is_none() {
                return Err(format!("{} is too large to repeat.", repeat_label(repeat)));
            }
        }
        Ok(Task {
            id: self.id,
            kind: self.kind,
            follow_up: self.follow_up,
            at,
            timeout_secs: self.timeout_secs,
            status: TaskStatus::Proposed,
            late: false,
            deadline: None,
            repeat: self.repeat,
        })
    }
}

pub fn schedule_definition() -> Value {
    json!({
        "name": "schedule",
        "description": "Propose a task that runs once at a later time, or repeats when every is given: either a shell command, or a worker that carries out a brief in its own worktree cut from origin/main at fire time. A scheduled worker never pushes. This tool only proposes: it records the task as awaiting approval and arms nothing. The chat itself asks the person to approve the exact command or brief and the time before anything is armed. Before calling it, tell the person what will run and when. When an approved task runs, its report reaches the inbox tagged schedule:<id>. Give exactly one of command or brief, and exactly one of at or after, unless every is given.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "Short name for the task, without spaces or slashes."},
                "command": {"type": "string", "description": "The exact shell command to run, as the person will approve it. Give this or brief."},
                "brief": {"type": "string", "description": "The full task for a scheduled worker, as the person will approve it. Give this or command."},
                "follow_up": {"type": "string", "description": "What to do when the command ends. Commands only."},
                "at": {"type": "string", "description": "When to run: an RFC 3339 timestamp, or a local time HH:MM meaning its next occurrence. Required unless every is given; give at most one of at or after."},
                "after": {"type": "string", "description": "How long to wait before running, such as 90s, 10m, 2h or 1h30m. Required unless every is given; give at most one of at or after."},
                "every": {"type": "string", "description": "Repeat the task: a duration such as 6h or 90m, or daily HH:MM in local time. With every, at and after are optional and set the first run; without them the first run is one interval later, or the next HH:MM. Minimum 10 minutes for a command and 60 minutes for a worker. It runs while the chat is on; a run missed while the chat was off fires once, late. Runs never overlap. A start failure stops the repetition. At most 5 repeating tasks. Approval is asked once, for all runs. /schedule cancel <id> stops it."},
                "timeout_secs": {"type": "integer", "minimum": 1, "description": "Stop the task after this many seconds. Default for a command: the configured watch timeout, 1 hour unless changed. Default for a worker: 2 hours; the maximum is 8 hours."}
            },
            "required": ["id"]
        }
    })
}

pub fn parse_schedule_args(args: &Value) -> Result<ScheduleRequest, String> {
    let id = string_arg(args, "id")?;
    if id
        .chars()
        .any(|c| c == '/' || c.is_whitespace() || c.is_control())
    {
        return Err(format!(
            "Schedule id '{}' must not contain spaces, slashes or control characters.",
            escape_controls(&id)
        ));
    }
    let follow_up = optional_string(args, "follow_up")?;
    let timeout_secs = timeout_arg(args)?;
    let kind = match (
        optional_string(args, "command")?,
        optional_string(args, "brief")?,
    ) {
        (Some(command), None) => TaskKind::Command { command },
        (None, Some(brief)) => {
            if follow_up.is_some() {
                return Err("follow_up applies to commands only.".to_string());
            }
            if !is_dashed_token(&id) {
                return Err(format!(
                    "Schedule id '{}' for a worker may only use letters, digits and dashes.",
                    escape_controls(&id)
                ));
            }
            if timeout_secs.is_some_and(|secs| secs > WORKER_MAX_TIMEOUT_SECS) {
                return Err(format!(
                    "timeout_secs for a worker must be at most {WORKER_MAX_TIMEOUT_SECS} (8 hours)."
                ));
            }
            TaskKind::Worker { brief }
        }
        _ => return Err("Give exactly one of command or brief.".to_string()),
    };
    let repeat = repeat_arg(args, &kind)?;
    Ok(ScheduleRequest {
        id,
        kind,
        follow_up,
        when: when_arg(args, repeat.is_some())?,
        repeat,
        timeout_secs,
    })
}

fn optional_string(args: &Value, key: &str) -> Result<Option<String>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => string_arg(args, key).map(Some),
    }
}

fn repeat_arg(args: &Value, kind: &TaskKind) -> Result<Option<Repeat>, String> {
    optional_string(args, "every")?
        .map(|raw| {
            let repeat = parse_repeat(&raw)?;
            check_min(&repeat, kind)?;
            Ok(repeat)
        })
        .transpose()
}

fn when_arg(args: &Value, repeating: bool) -> Result<Option<When>, String> {
    match (
        optional_string(args, "at")?,
        optional_string(args, "after")?,
    ) {
        (Some(at), None) => Ok(Some(When::At(at))),
        (None, Some(after)) => parse_after(&after).map(|span| Some(When::After(span))),
        (None, None) if repeating => Ok(None),
        _ if repeating => Err("Give at most one of at or after with every.".to_string()),
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
    format!("schedule `{id}` proposed; the chat will ask the person to approve it; do not suggest a reply to approve it")
}

pub fn duplicate_message(id: &str) -> String {
    format!("schedule `{id}` already exists")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-01-01T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn offset() -> FixedOffset {
        FixedOffset::east_opt(2 * 3600).unwrap()
    }

    fn command_kind(command: &str) -> TaskKind {
        TaskKind::Command {
            command: command.to_string(),
        }
    }

    fn worker_request(extra: Value) -> Result<ScheduleRequest, String> {
        let mut args = json!({"id": "nightly", "brief": "fix it", "after": "5m"});
        for (key, value) in extra.as_object().unwrap() {
            args[key] = value.clone();
        }
        parse_schedule_args(&args)
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
                kind: command_kind("make test"),
                follow_up: None,
                when: Some(When::After(Duration::from_secs(600))),
                repeat: None,
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
                kind: command_kind("make test"),
                follow_up: Some("report".to_string()),
                when: Some(When::At("14:30".to_string())),
                repeat: None,
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
    fn an_id_with_a_control_character_is_refused() {
        for id in ["a\u{1b}[2K", "a\rb", "a\u{7f}"] {
            let error = request(json!({"after": "5m", "id": id})).unwrap_err();
            assert!(error.contains("control characters"), "{error}");
            assert!(
                !error.contains('\u{1b}') && !error.contains('\r'),
                "{error:?}"
            );
        }
    }

    #[test]
    fn a_past_rfc3339_at_fails_when_resolved() {
        let error = request(json!({"at": "2026-01-01T09:00:00Z"}))
            .unwrap()
            .into_task(now(), &offset())
            .unwrap_err();
        assert!(error.contains("is in the past"), "{error}");
    }

    #[test]
    fn after_resolves_from_now_into_a_proposed_task() {
        let task = request(json!({"after": "1h30m", "timeout_secs": 7}))
            .unwrap()
            .into_task(now(), &offset())
            .unwrap();
        assert_eq!(task.id, "nightly");
        assert_eq!(task.kind, command_kind("make test"));
        assert_eq!(task.at, now() + ChronoDuration::minutes(90));
        assert_eq!(task.status, TaskStatus::Proposed);
        assert_eq!(task.timeout_secs, Some(7));
        assert!(!task.late);
    }

    #[test]
    fn at_resolves_in_the_local_offset() {
        let task = request(json!({"at": "14:30"}))
            .unwrap()
            .into_task(now(), &offset())
            .unwrap();
        assert_eq!(task.at, now() + ChronoDuration::minutes(150));
    }

    #[test]
    fn an_unreadable_at_fails_when_resolved() {
        assert!(request(json!({"at": "later"}))
            .unwrap()
            .into_task(now(), &offset())
            .is_err());
    }

    #[test]
    fn an_overflowing_after_fails_when_resolved() {
        let huge = ScheduleRequest {
            id: "a".to_string(),
            kind: command_kind("x"),
            follow_up: None,
            when: Some(When::After(Duration::from_secs(u64::MAX))),
            repeat: None,
            timeout_secs: None,
        };
        assert!(huge.into_task(now(), &offset()).is_err());
    }

    #[test]
    fn the_definition_requires_only_id_and_says_it_only_proposes() {
        let definition = schedule_definition();
        assert_eq!(definition["name"], "schedule");
        assert_eq!(definition["inputSchema"]["required"], json!(["id"]));
        let description = definition["description"].as_str().unwrap();
        assert!(description.contains("only proposes"));
        assert!(description.contains("asks the person to approve"));
        for key in [
            "command",
            "brief",
            "follow_up",
            "at",
            "after",
            "every",
            "timeout_secs",
        ] {
            assert!(definition["inputSchema"]["properties"][key].is_object());
        }
    }

    #[test]
    fn a_worker_request_carries_its_brief() {
        assert_eq!(
            worker_request(json!({})),
            Ok(ScheduleRequest {
                id: "nightly".to_string(),
                kind: TaskKind::Worker {
                    brief: "fix it".to_string()
                },
                follow_up: None,
                when: Some(When::After(Duration::from_secs(300))),
                repeat: None,
                timeout_secs: None,
            })
        );
    }

    #[test]
    fn a_worker_id_follows_the_dashed_token_rule_but_a_command_id_does_not() {
        assert!(worker_request(json!({"id": "night_ly"})).is_err());
        assert!(worker_request(json!({"id": "night.ly"})).is_err());
        assert!(worker_request(json!({"id": "night-ly"})).is_ok());
        assert!(parse_schedule_args(&json!({
            "id": "night_ly",
            "command": "make test",
            "after": "5m"
        }))
        .is_ok());
    }

    #[test]
    fn exactly_one_of_command_and_brief_is_required() {
        assert!(worker_request(json!({"command": "make test"})).is_err());
        assert!(request(json!({"after": "5m", "command": null})).is_err());
        let neither = parse_schedule_args(&json!({"id": "a", "after": "5m"}));
        assert_eq!(
            neither.unwrap_err(),
            "Give exactly one of command or brief."
        );
        assert!(worker_request(json!({"brief": " "})).is_err());
    }

    #[test]
    fn a_worker_timeout_is_capped_at_eight_hours() {
        assert!(worker_request(json!({"timeout_secs": 28800})).is_ok());
        let error = worker_request(json!({"timeout_secs": 28801})).unwrap_err();
        assert!(error.contains("at most 28800"), "{error}");
        assert!(request(json!({"after": "5m", "timeout_secs": 28801})).is_ok());
    }

    #[test]
    fn a_worker_request_refuses_a_follow_up() {
        assert!(worker_request(json!({"follow_up": "report"})).is_err());
    }

    #[test]
    fn a_worker_request_resolves_into_a_proposed_worker_task() {
        let task = worker_request(json!({}))
            .unwrap()
            .into_task(now(), &offset())
            .unwrap();
        assert_eq!(
            task.kind,
            TaskKind::Worker {
                brief: "fix it".to_string()
            }
        );
        assert_eq!(task.status, TaskStatus::Proposed);
    }

    fn every_request(args: Value) -> Result<ScheduleRequest, String> {
        parse_schedule_args(&args)
    }

    #[test]
    fn every_parses_for_a_command() {
        let parsed = every_request(json!({"id": "a", "command": "ls", "every": "30m"})).unwrap();
        assert_eq!(parsed.repeat, Some(Repeat::Every { every_secs: 1800 }));
        assert_eq!(parsed.when, None);
    }

    #[test]
    fn every_parses_for_a_worker() {
        let parsed = every_request(json!({"id": "a", "brief": "b", "every": "2h"})).unwrap();
        assert_eq!(parsed.repeat, Some(Repeat::Every { every_secs: 7200 }));
    }

    #[test]
    fn every_daily_parses() {
        let parsed =
            every_request(json!({"id": "a", "command": "ls", "every": "daily 02:00"})).unwrap();
        assert_eq!(
            parsed.repeat,
            Some(Repeat::Daily {
                daily: "02:00".to_string()
            })
        );
    }

    #[test]
    fn every_below_the_minimum_is_rejected() {
        assert!(
            every_request(json!({"id": "a", "command": "ls", "every": "5m", "after": "5m"}))
                .is_err()
        );
        assert!(
            every_request(json!({"id": "a", "brief": "b", "every": "30m", "after": "5m"})).is_err()
        );
    }

    #[test]
    fn every_garbage_is_rejected() {
        assert!(
            every_request(json!({"id": "a", "command": "ls", "every": "soon", "after": "5m"}))
                .is_err()
        );
        assert!(every_request(
            json!({"id": "a", "command": "ls", "every": "daily 25:99", "after": "5m"})
        )
        .is_err());
    }

    #[test]
    fn every_accepts_one_of_at_and_after() {
        let parsed =
            every_request(json!({"id": "a", "command": "ls", "every": "1h", "at": "14:30"}))
                .unwrap();
        assert_eq!(parsed.when, Some(When::At("14:30".to_string())));
        assert!(parsed.repeat.is_some());
        let parsed =
            every_request(json!({"id": "a", "command": "ls", "every": "1h", "after": "5m"}))
                .unwrap();
        assert_eq!(parsed.when, Some(When::After(Duration::from_secs(300))));
    }

    #[test]
    fn every_with_both_at_and_after_is_rejected() {
        let error = every_request(
            json!({"id": "a", "command": "ls", "every": "1h", "at": "14:30", "after": "5m"}),
        )
        .unwrap_err();
        assert_eq!(error, "Give at most one of at or after with every.");
    }

    #[test]
    fn neither_every_nor_when_keeps_the_old_message() {
        let error = every_request(json!({"id": "a", "command": "ls"})).unwrap_err();
        assert_eq!(error, "Give exactly one of at or after.");
        let error =
            every_request(json!({"id": "a", "command": "ls", "at": "14:30", "after": "5m"}))
                .unwrap_err();
        assert_eq!(error, "Give exactly one of at or after.");
    }

    fn repeating(repeat: Repeat) -> ScheduleRequest {
        ScheduleRequest {
            id: "a".to_string(),
            kind: command_kind("ls"),
            follow_up: None,
            when: None,
            repeat: Some(repeat),
            timeout_secs: None,
        }
    }

    #[test]
    fn into_task_with_every_starts_one_interval_from_now() {
        let task = repeating(Repeat::Every { every_secs: 3600 })
            .into_task(now(), &offset())
            .unwrap();
        assert_eq!(task.at, now() + ChronoDuration::hours(1));
        assert_eq!(task.repeat, Some(Repeat::Every { every_secs: 3600 }));
        assert_eq!(task.status, TaskStatus::Proposed);
    }

    #[test]
    fn into_task_with_daily_ahead_starts_today() {
        let task = repeating(Repeat::Daily {
            daily: "14:30".to_string(),
        })
        .into_task(now(), &offset())
        .unwrap();
        assert_eq!(task.at, now() + ChronoDuration::minutes(150));
        assert_eq!(task.status, TaskStatus::Proposed);
    }

    #[test]
    fn into_task_with_daily_passed_starts_tomorrow() {
        let task = repeating(Repeat::Daily {
            daily: "09:00".to_string(),
        })
        .into_task(now(), &offset())
        .unwrap();
        assert_eq!(task.at, now() + ChronoDuration::hours(21));
    }

    #[test]
    fn into_task_with_daily_and_after_runs_as_given() {
        let mut request = repeating(Repeat::Daily {
            daily: "02:00".to_string(),
        });
        request.when = Some(When::After(Duration::from_secs(600)));
        let task = request.into_task(now(), &offset()).unwrap();
        assert_eq!(task.at, now() + ChronoDuration::minutes(10));
        assert!(task.repeat.is_some());
    }

    #[test]
    fn into_task_refuses_an_every_too_large_to_repeat_after_the_first_run() {
        for when in [
            When::After(Duration::from_secs(600)),
            When::At("14:30".to_string()),
        ] {
            let mut request = repeating(Repeat::Every {
                every_secs: u64::MAX,
            });
            request.when = Some(when);
            let error = request.into_task(now(), &offset()).unwrap_err();
            assert!(error.contains("is too large to repeat"), "{error}");
        }
    }

    #[test]
    fn into_task_accepts_a_normal_every_with_a_first_run() {
        let mut request = repeating(Repeat::Every { every_secs: 3600 });
        request.when = Some(When::At("14:30".to_string()));
        let task = request.into_task(now(), &offset()).unwrap();
        assert_eq!(task.at, now() + ChronoDuration::minutes(150));
    }

    #[test]
    fn into_task_accepts_daily_with_a_first_run() {
        let mut request = repeating(Repeat::Daily {
            daily: "02:00".to_string(),
        });
        request.when = Some(When::At("14:30".to_string()));
        assert!(request.into_task(now(), &offset()).is_ok());
    }

    #[test]
    fn the_every_description_states_its_rules() {
        let definition = schedule_definition();
        let every = definition["inputSchema"]["properties"]["every"]["description"]
            .as_str()
            .unwrap();
        for needle in [
            "daily HH:MM",
            "10 minutes",
            "60 minutes",
            "/schedule cancel",
        ] {
            assert!(every.contains(needle), "{needle}");
        }
    }

    #[test]
    fn messages_name_the_task() {
        assert!(proposed_message("a").contains("`a`"));
        assert!(proposed_message("a").contains("approve"));
        assert!(proposed_message("a").ends_with("; do not suggest a reply to approve it"));
        assert!(duplicate_message("a").contains("`a`"));
    }
}
