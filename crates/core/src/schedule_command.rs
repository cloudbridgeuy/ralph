use crate::schedule::{
    escape_controls, format_span, watch_id, Task, TaskKind, TaskStatus, SCHEDULE_ID_PREFIX,
};
use chrono::{DateTime, FixedOffset, Utc};

pub const SCHEDULE_USAGE: &str = "Usage: /schedule [list] | cancel <id>. /schedule does not create tasks: the model proposes them and you arm each one at a y/n prompt.";
const NO_TASKS: &str = "No scheduled tasks.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleCommand {
    List,
    Cancel(String),
    Usage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CancelPlan {
    Remove {
        id: String,
        stop_watch: Option<String>,
    },
    Unknown(String),
}

pub fn parse_schedule_args(args: &str) -> ScheduleCommand {
    let words: Vec<&str> = args.split_whitespace().collect();
    match words.as_slice() {
        [] | ["list"] => ScheduleCommand::List,
        ["cancel", id] => match id.strip_prefix(SCHEDULE_ID_PREFIX).unwrap_or(id) {
            "" => ScheduleCommand::Usage,
            id => ScheduleCommand::Cancel(id.to_string()),
        },
        _ => ScheduleCommand::Usage,
    }
}

pub fn plan_cancel(id: &str, tasks: &[Task]) -> CancelPlan {
    match tasks.iter().find(|task| task.id == id) {
        Some(task) => CancelPlan::Remove {
            id: task.id.clone(),
            stop_watch: (task.status == TaskStatus::Running).then(|| watch_id(&task.id)),
        },
        None => CancelPlan::Unknown(id.to_string()),
    }
}

pub fn cancel_message(plan: &CancelPlan) -> String {
    match plan {
        CancelPlan::Remove {
            id,
            stop_watch: None,
        } => format!("Cancelled scheduled task {id}."),
        CancelPlan::Remove {
            id,
            stop_watch: Some(_),
        } => format!("Cancelled scheduled task {id} and stopped its running watch."),
        CancelPlan::Unknown(id) => {
            format!("No scheduled task {id}. /schedule list shows the tasks.")
        }
    }
}

pub fn status_label(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Proposed => "proposed",
        TaskStatus::Pending => "armed",
        TaskStatus::Running => "running",
    }
}

fn when_label(task: &Task, now: DateTime<Utc>, offset: FixedOffset) -> String {
    let local = task.at.with_timezone(&offset);
    if local.date_naive() == now.with_timezone(&offset).date_naive() {
        local.format("%H:%M").to_string()
    } else {
        local.format("%Y-%m-%d %H:%M").to_string()
    }
}

fn relative_label(task: &Task, now: DateTime<Utc>) -> String {
    let remaining = (task.at - now).num_seconds();
    let relative = if remaining > 0 {
        format!("in {}", format_span(remaining.unsigned_abs()))
    } else {
        "due".to_string()
    };
    if task.late {
        format!("{relative} late")
    } else {
        relative
    }
}

fn task_text(task: &Task) -> &str {
    match &task.kind {
        TaskKind::Command { command } => command,
        TaskKind::Worker { brief } => brief.lines().next().unwrap_or_default(),
    }
}

pub fn list_tasks(tasks: &[Task], now: DateTime<Utc>, offset: FixedOffset) -> String {
    if tasks.is_empty() {
        return NO_TASKS.to_string();
    }
    let mut sorted: Vec<&Task> = tasks.iter().collect();
    sorted.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.id.cmp(&b.id)));
    let rows: Vec<[String; 4]> = sorted
        .iter()
        .map(|task| {
            [
                escape_controls(&task.id),
                status_label(task.status).to_string(),
                when_label(task, now, offset),
                relative_label(task, now),
            ]
        })
        .collect();
    let widths: Vec<usize> = (0..4)
        .map(|column| {
            rows.iter()
                .map(|row| row[column].chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    rows.iter()
        .zip(&sorted)
        .map(|(row, task)| {
            format!(
                "{:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}  {}",
                row[0],
                row[1],
                row[2],
                row[3],
                escape_controls(task_text(task)),
                w0 = widths[0],
                w1 = widths[1],
                w2 = widths[2],
                w3 = widths[3],
            )
        })
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

    fn task(id: &str, at: &str, status: TaskStatus, late: bool) -> Task {
        Task {
            id: id.into(),
            kind: TaskKind::Command {
                command: format!("echo {id}"),
            },
            follow_up: None,
            at: utc(at),
            timeout_secs: None,
            status,
            late,
        }
    }

    #[test]
    fn bare_and_list_mean_list() {
        assert_eq!(parse_schedule_args(""), ScheduleCommand::List);
        assert_eq!(parse_schedule_args("  list "), ScheduleCommand::List);
    }

    #[test]
    fn cancel_takes_a_bare_or_prefixed_id() {
        let cancel = |id: &str| ScheduleCommand::Cancel(id.to_string());
        assert_eq!(parse_schedule_args("cancel nightly"), cancel("nightly"));
        assert_eq!(
            parse_schedule_args("cancel schedule:nightly"),
            cancel("nightly")
        );
    }

    #[test]
    fn other_input_is_usage() {
        for args in [
            "cancel",
            "cancel a b",
            "cancel schedule:",
            "list now",
            "start x",
            "x",
        ] {
            assert_eq!(parse_schedule_args(args), ScheduleCommand::Usage, "{args}");
        }
    }

    #[test]
    fn usage_says_the_command_does_not_create_tasks() {
        assert!(SCHEDULE_USAGE.contains("does not create"));
    }

    #[test]
    fn cancel_plan_stops_the_watch_only_for_a_running_task() {
        let tasks = [
            task("a", "2026-01-01T10:00:00Z", TaskStatus::Pending, false),
            task("b", "2026-01-01T10:00:00Z", TaskStatus::Running, false),
        ];
        assert_eq!(
            plan_cancel("a", &tasks),
            CancelPlan::Remove {
                id: "a".into(),
                stop_watch: None
            }
        );
        assert_eq!(
            plan_cancel("b", &tasks),
            CancelPlan::Remove {
                id: "b".into(),
                stop_watch: Some("schedule:b".into())
            }
        );
        assert_eq!(plan_cancel("c", &tasks), CancelPlan::Unknown("c".into()));
    }

    #[test]
    fn cancel_messages_differ_by_plan() {
        let plain = cancel_message(&plan_cancel(
            "a",
            &[task(
                "a",
                "2026-01-01T10:00:00Z",
                TaskStatus::Pending,
                false,
            )],
        ));
        let running = cancel_message(&plan_cancel(
            "a",
            &[task(
                "a",
                "2026-01-01T10:00:00Z",
                TaskStatus::Running,
                false,
            )],
        ));
        let unknown = cancel_message(&plan_cancel("z", &[]));
        assert_eq!(plain, "Cancelled scheduled task a.");
        assert_eq!(
            running,
            "Cancelled scheduled task a and stopped its running watch."
        );
        assert_eq!(
            unknown,
            "No scheduled task z. /schedule list shows the tasks."
        );
    }

    #[test]
    fn status_labels_name_each_state() {
        assert_eq!(status_label(TaskStatus::Proposed), "proposed");
        assert_eq!(status_label(TaskStatus::Pending), "armed");
        assert_eq!(status_label(TaskStatus::Running), "running");
    }

    #[test]
    fn empty_list_says_so() {
        assert_eq!(
            list_tasks(&[], utc("2026-01-01T10:00:00Z"), offset()),
            "No scheduled tasks."
        );
    }

    #[test]
    fn list_sorts_by_time_and_shows_local_time_relative_and_late() {
        let now = utc("2026-01-01T10:00:00Z");
        let tasks = [
            task("later", "2026-01-01T11:30:00Z", TaskStatus::Pending, false),
            task("soon", "2026-01-01T10:05:00Z", TaskStatus::Proposed, false),
            task("missed", "2026-01-01T09:00:00Z", TaskStatus::Pending, true),
        ];
        let text = list_tasks(&tasks, now, offset());
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines,
            [
                "missed  armed     11:00  due late  echo missed",
                "soon    proposed  12:05  in 5m     echo soon",
                "later   armed     13:30  in 1h30m  echo later",
            ]
        );
    }

    #[test]
    fn list_adds_the_date_for_another_day() {
        let now = utc("2026-01-01T10:00:00Z");
        let tasks = [task(
            "t",
            "2026-01-02T08:00:00Z",
            TaskStatus::Running,
            false,
        )];
        assert_eq!(
            list_tasks(&tasks, now, offset()),
            "t  running  2026-01-02 10:00  in 22h  echo t"
        );
    }

    #[test]
    fn a_task_due_at_the_same_second_is_due_not_in_zero() {
        let now = utc("2026-01-01T10:00:00Z");
        let tasks = [task(
            "t",
            "2026-01-01T10:00:00Z",
            TaskStatus::Pending,
            false,
        )];
        assert!(list_tasks(&tasks, now, offset()).contains("due"));
    }

    #[test]
    fn list_shows_escape_codes_in_the_command() {
        let now = utc("2026-01-01T10:00:00Z");
        let mut t = task("t", "2026-01-01T10:00:00Z", TaskStatus::Pending, false);
        t.kind = TaskKind::Command {
            command: "rm -rf ~\x1b[2K\recho ok".to_string(),
        };
        let text = list_tasks(&[t], now, offset());
        assert!(!text.contains('\x1b') && !text.contains('\r'), "{text:?}");
        assert!(text.ends_with("rm -rf ~\\x1b[2K\\recho ok"), "{text}");
    }

    #[test]
    fn list_shows_the_first_line_of_a_worker_brief() {
        let now = utc("2026-01-01T10:00:00Z");
        let mut t = task("w", "2026-01-01T10:05:00Z", TaskStatus::Pending, false);
        t.kind = TaskKind::Worker {
            brief: "fix the build\nthen the tests".to_string(),
        };
        assert_eq!(
            list_tasks(&[t], now, offset()),
            "w  armed  12:05  in 5m  fix the build"
        );
    }
}
