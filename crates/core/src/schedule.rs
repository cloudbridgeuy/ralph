use crate::watch::{WatchKind, WatchSpec};
use chrono::{DateTime, Duration as ChronoDuration, FixedOffset, NaiveTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const SCHEDULE_ID_PREFIX: &str = "schedule:";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Proposed,
    Pending,
    Running,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub kind: TaskKind,
    pub command: String,
    pub follow_up: Option<String>,
    pub at: DateTime<Utc>,
    pub timeout_secs: Option<u64>,
    pub status: TaskStatus,
    pub late: bool,
}

pub fn parse_after(raw: &str) -> Result<Duration, String> {
    let text = raw.trim();
    if text.is_empty() {
        return Err("after must not be empty.".to_string());
    }
    let mut rest = text;
    let mut total: u64 = 0;
    let mut previous_unit = u64::MAX;
    while !rest.is_empty() {
        let digits_end = rest
            .find(|c: char| !c.is_ascii_digit())
            .ok_or_else(|| format!("after '{raw}' is missing a unit (h, m or s)."))?;
        let (digits, tail) = rest.split_at(digits_end);
        let amount: u64 = digits
            .parse()
            .map_err(|_| format!("after '{raw}' must use whole numbers like 1h30m."))?;
        let mut chars = tail.chars();
        let unit = match chars.next() {
            Some('h') => 3600,
            Some('m') => 60,
            Some('s') => 1,
            Some(other) => return Err(format!("after '{raw}' has unknown unit '{other}'.")),
            None => return Err(format!("after '{raw}' is missing a unit (h, m or s).")),
        };
        if unit >= previous_unit {
            return Err(format!(
                "after '{raw}' must list each unit once, largest first."
            ));
        }
        previous_unit = unit;
        total = amount
            .checked_mul(unit)
            .and_then(|secs| total.checked_add(secs))
            .ok_or_else(|| format!("after '{raw}' is too large."))?;
        rest = chars.as_str();
    }
    if total == 0 {
        return Err("after must be longer than zero.".to_string());
    }
    Ok(Duration::from_secs(total))
}

pub fn parse_at(
    raw: &str,
    now: DateTime<Utc>,
    offset: FixedOffset,
) -> Result<DateTime<Utc>, String> {
    let text = raw.trim();
    if let Ok(moment) = DateTime::parse_from_rfc3339(text) {
        return Ok(moment.with_timezone(&Utc));
    }
    let time = NaiveTime::parse_from_str(text, "%H:%M")
        .map_err(|_| format!("at '{raw}' must be an RFC 3339 timestamp or a local HH:MM time."))?;
    let local_now = now.with_timezone(&offset);
    let today = local_now.date_naive();
    [today, today + ChronoDuration::days(1)]
        .into_iter()
        .filter_map(|day| offset.from_local_datetime(&day.and_time(time)).single())
        .map(|moment| moment.with_timezone(&Utc))
        .find(|moment| *moment > now)
        .ok_or_else(|| format!("at '{raw}' is not a valid local time."))
}

pub fn due(tasks: &[Task], now: DateTime<Utc>) -> Vec<&Task> {
    tasks
        .iter()
        .filter(|task| task.status == TaskStatus::Pending && task.at <= now)
        .collect()
}

pub fn plan_restore(tasks: Vec<Task>, now: DateTime<Utc>) -> Vec<Task> {
    tasks
        .into_iter()
        .map(|task| match task.status {
            TaskStatus::Running => Task {
                status: TaskStatus::Pending,
                late: task.late || task.at < now,
                ..task
            },
            TaskStatus::Pending => Task {
                late: task.late || task.at < now,
                ..task
            },
            TaskStatus::Proposed => task,
        })
        .collect()
}

pub fn watch_id(id: &str) -> String {
    format!("{SCHEDULE_ID_PREFIX}{id}")
}

pub fn to_watch_spec(task: &Task, now: DateTime<Utc>, offset: FixedOffset) -> WatchSpec {
    let timeout = task.timeout_secs.map(Duration::from_secs);
    WatchSpec {
        id: watch_id(&task.id),
        command: task.command.clone(),
        follow_up: follow_up(task, now, offset),
        kind: WatchKind::Normal,
        timeout,
        limit: timeout,
    }
}

fn follow_up(task: &Task, now: DateTime<Utc>, offset: FixedOffset) -> Option<String> {
    if !task.late {
        return task.follow_up.clone();
    }
    let notice = format!(
        "Ran late: due {}, started {}.",
        task.at.with_timezone(&offset).format("%H:%M"),
        now.with_timezone(&offset).format("%H:%M"),
    );
    Some(match &task.follow_up {
        Some(text) => format!("{notice}\n{text}"),
        None => notice,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn offset() -> FixedOffset {
        FixedOffset::east_opt(2 * 3600).unwrap()
    }

    fn task(id: &str, at: &str, status: TaskStatus) -> Task {
        Task {
            id: id.to_string(),
            kind: TaskKind::Command,
            command: "echo hi".to_string(),
            follow_up: None,
            at: utc(at),
            timeout_secs: None,
            status,
            late: false,
        }
    }

    #[test]
    fn after_parses_units_and_combinations() {
        assert_eq!(parse_after("90s").unwrap(), Duration::from_secs(90));
        assert_eq!(parse_after("10m").unwrap(), Duration::from_secs(600));
        assert_eq!(parse_after("2h").unwrap(), Duration::from_secs(7200));
        assert_eq!(parse_after("1h30m").unwrap(), Duration::from_secs(5400));
        assert_eq!(parse_after("1h30m15s").unwrap(), Duration::from_secs(5415));
        assert_eq!(parse_after(" 5m ").unwrap(), Duration::from_secs(300));
    }

    #[test]
    fn after_rejects_bad_input() {
        for bad in [
            "",
            "  ",
            "0s",
            "0h0m",
            "5",
            "m",
            "5d",
            "1s1m",
            "1m1m",
            "-5m",
            "1.5h",
            "5 m",
            "h5",
            "99999999999999999999s",
            "18446744073709551615h",
        ] {
            assert!(parse_after(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn at_accepts_rfc3339() {
        let now = utc("2026-01-01T10:00:00Z");
        assert_eq!(
            parse_at("2026-01-02T12:00:00+02:00", now, offset()).unwrap(),
            utc("2026-01-02T10:00:00Z")
        );
    }

    #[test]
    fn at_clock_time_today_when_still_ahead() {
        let now = utc("2026-01-01T10:00:00Z");
        assert_eq!(
            parse_at("14:30", now, offset()).unwrap(),
            utc("2026-01-01T12:30:00Z")
        );
    }

    #[test]
    fn at_clock_time_tomorrow_when_passed_or_equal() {
        let now = utc("2026-01-01T10:00:00Z");
        assert_eq!(
            parse_at("09:00", now, offset()).unwrap(),
            utc("2026-01-02T07:00:00Z")
        );
        assert_eq!(
            parse_at("12:00", now, offset()).unwrap(),
            utc("2026-01-02T10:00:00Z")
        );
    }

    #[test]
    fn at_uses_the_local_date_not_the_utc_date() {
        let now = utc("2026-01-01T23:00:00Z");
        assert_eq!(
            parse_at("02:00", now, offset()).unwrap(),
            utc("2026-01-02T00:00:00Z")
        );
    }

    #[test]
    fn at_rejects_garbage() {
        let now = utc("2026-01-01T10:00:00Z");
        for bad in ["", "tomorrow", "25:00", "12:60", "2026-01-01"] {
            assert!(parse_at(bad, now, offset()).is_err(), "{bad}");
        }
    }

    #[test]
    fn due_returns_only_pending_tasks_at_or_before_now() {
        let now = utc("2026-01-01T10:00:00Z");
        let tasks = vec![
            task("past", "2026-01-01T09:00:00Z", TaskStatus::Pending),
            task("exact", "2026-01-01T10:00:00Z", TaskStatus::Pending),
            task("future", "2026-01-01T10:00:01Z", TaskStatus::Pending),
            task("proposed", "2026-01-01T09:00:00Z", TaskStatus::Proposed),
            task("running", "2026-01-01T09:00:00Z", TaskStatus::Running),
        ];
        let ids: Vec<&str> = due(&tasks, now).iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["past", "exact"]);
    }

    #[test]
    fn restore_requeues_running_and_marks_overdue_late() {
        let now = utc("2026-01-01T10:00:00Z");
        let restored = plan_restore(
            vec![
                task("run-past", "2026-01-01T09:00:00Z", TaskStatus::Running),
                task("run-future", "2026-01-01T11:00:00Z", TaskStatus::Running),
                task("pend-past", "2026-01-01T09:00:00Z", TaskStatus::Pending),
                task("pend-now", "2026-01-01T10:00:00Z", TaskStatus::Pending),
                task("pend-future", "2026-01-01T11:00:00Z", TaskStatus::Pending),
                task("prop-past", "2026-01-01T09:00:00Z", TaskStatus::Proposed),
            ],
            now,
        );
        let view: Vec<(TaskStatus, bool)> = restored.iter().map(|t| (t.status, t.late)).collect();
        assert_eq!(
            view,
            vec![
                (TaskStatus::Pending, true),
                (TaskStatus::Pending, false),
                (TaskStatus::Pending, true),
                (TaskStatus::Pending, false),
                (TaskStatus::Pending, false),
                (TaskStatus::Proposed, false),
            ]
        );
    }

    #[test]
    fn restore_keeps_an_existing_late_flag() {
        let mut already = task("x", "2026-01-01T11:00:00Z", TaskStatus::Pending);
        already.late = true;
        let restored = plan_restore(vec![already], utc("2026-01-01T10:00:00Z"));
        assert!(restored[0].late);
    }

    #[test]
    fn watch_id_has_the_schedule_prefix() {
        assert_eq!(watch_id("nightly"), "schedule:nightly");
    }

    #[test]
    fn spec_is_never_persistent() {
        let now = utc("2026-01-01T10:00:00Z");
        let spec = to_watch_spec(
            &task("a", "2026-01-01T10:00:00Z", TaskStatus::Pending),
            now,
            offset(),
        );
        assert_eq!(spec.kind, WatchKind::Normal);
    }

    #[test]
    fn spec_carries_command_id_and_timeout() {
        let now = utc("2026-01-01T10:00:00Z");
        let mut t = task("a", "2026-01-01T10:00:00Z", TaskStatus::Pending);
        t.follow_up = Some("summarise".to_string());
        t.timeout_secs = Some(30);
        let spec = to_watch_spec(&t, now, offset());
        assert_eq!(spec.id, "schedule:a");
        assert_eq!(spec.command, "echo hi");
        assert_eq!(spec.follow_up.as_deref(), Some("summarise"));
        assert_eq!(spec.timeout, Some(Duration::from_secs(30)));
        assert_eq!(spec.limit, Some(Duration::from_secs(30)));
    }

    #[test]
    fn spec_without_timeout_leaves_the_default_to_the_caller() {
        let now = utc("2026-01-01T10:00:00Z");
        let spec = to_watch_spec(
            &task("a", "2026-01-01T10:00:00Z", TaskStatus::Pending),
            now,
            offset(),
        );
        assert_eq!(spec.timeout, None);
        assert_eq!(spec.limit, None);
        assert_eq!(spec.follow_up, None);
    }

    #[test]
    fn late_spec_prefixes_the_follow_up_in_local_time() {
        let now = utc("2026-01-01T10:05:00Z");
        let mut t = task("a", "2026-01-01T08:30:00Z", TaskStatus::Pending);
        t.late = true;
        let bare = to_watch_spec(&t, now, offset());
        assert_eq!(
            bare.follow_up.as_deref(),
            Some("Ran late: due 10:30, started 12:05.")
        );
        t.follow_up = Some("summarise".to_string());
        let with = to_watch_spec(&t, now, offset());
        assert_eq!(
            with.follow_up.as_deref(),
            Some("Ran late: due 10:30, started 12:05.\nsummarise")
        );
    }

    #[test]
    fn task_round_trips_through_json() {
        let t = task("a", "2026-01-01T10:00:00Z", TaskStatus::Proposed);
        let back: Task = serde_json::from_slice(&serde_json::to_vec(&t).unwrap()).unwrap();
        assert_eq!(back, t);
    }
}
