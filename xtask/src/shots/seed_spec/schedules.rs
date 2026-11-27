use super::at;
use ralph_core::schedule::{Task, TaskKind, TaskStatus, SCHEDULE_ID_PREFIX};
use ralph_core::schedule_repeat::Repeat;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleKind {
    Once,
    Every,
    Daily,
    Due,
    Late,
    Running,
    Proposed,
    WorkerTimeout,
}

#[cfg(test)]
impl ScheduleKind {
    pub const ALL: [ScheduleKind; 8] = [
        Self::Once,
        Self::Every,
        Self::Daily,
        Self::Due,
        Self::Late,
        Self::Running,
        Self::Proposed,
        Self::WorkerTimeout,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleSpec {
    pub kind: ScheduleKind,
    pub task: Task,
}

fn command(text: &str) -> TaskKind {
    TaskKind::Command {
        command: text.to_string(),
    }
}

fn brief(text: &str) -> TaskKind {
    TaskKind::Worker {
        brief: text.to_string(),
    }
}

fn pending(name: &str, kind: TaskKind, offset: i64) -> Task {
    Task {
        id: format!("{SCHEDULE_ID_PREFIX}{name}"),
        kind,
        follow_up: None,
        at: at(offset),
        timeout_secs: None,
        status: TaskStatus::Pending,
        late: false,
        deadline: None,
        repeat: None,
    }
}

pub fn schedule_specs() -> Vec<ScheduleSpec> {
    let spec = |kind, task| ScheduleSpec { kind, task };
    vec![
        spec(
            ScheduleKind::Once,
            pending("demo-once", command("ls demo"), 7_200),
        ),
        spec(
            ScheduleKind::Every,
            Task {
                repeat: Some(Repeat::Every { every_secs: 3_600 }),
                ..pending("demo-every", command("sh demo-check.sh"), 3_600)
            },
        ),
        spec(
            ScheduleKind::Daily,
            Task {
                repeat: Some(Repeat::Daily {
                    daily: "02:00".to_string(),
                }),
                ..pending(
                    "demo-daily",
                    brief("Write a short made-up status note."),
                    50_400,
                )
            },
        ),
        spec(
            ScheduleKind::Due,
            pending("demo-due", command("ls demo-due"), -300),
        ),
        spec(
            ScheduleKind::Late,
            Task {
                late: true,
                ..pending("demo-late", command("ls demo-late"), -7_200)
            },
        ),
        spec(
            ScheduleKind::Running,
            Task {
                status: TaskStatus::Running,
                timeout_secs: Some(3_600),
                deadline: Some(at(1_800)),
                ..pending("demo-running", command("sh demo-long.sh"), -1_800)
            },
        ),
        spec(
            ScheduleKind::Proposed,
            Task {
                status: TaskStatus::Proposed,
                follow_up: Some("Demo follow-up: read the result.".to_string()),
                ..pending("demo-proposed", command("ls demo-next"), 86_400)
            },
        ),
        spec(
            ScheduleKind::WorkerTimeout,
            Task {
                timeout_secs: Some(7_200),
                deadline: Some(at(9_000)),
                ..pending("demo-worker", brief("Tidy the made-up notes."), 1_800)
            },
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_lists_every_schedule_kind_variant_once() {
        let index = |value: ScheduleKind| match value {
            ScheduleKind::Once => 0,
            ScheduleKind::Every => 1,
            ScheduleKind::Daily => 2,
            ScheduleKind::Due => 3,
            ScheduleKind::Late => 4,
            ScheduleKind::Running => 5,
            ScheduleKind::Proposed => 6,
            ScheduleKind::WorkerTimeout => 7,
        };
        let indexes: Vec<usize> = ScheduleKind::ALL
            .iter()
            .map(|value| index(*value))
            .collect();
        assert_eq!(indexes, (0..8).collect::<Vec<_>>());
    }

    #[test]
    fn every_schedule_kind_has_one_task() {
        let specs = schedule_specs();
        for kind in ScheduleKind::ALL {
            assert_eq!(
                specs.iter().filter(|spec| spec.kind == kind).count(),
                1,
                "{kind:?}"
            );
        }
        assert_eq!(specs.len(), ScheduleKind::ALL.len());
    }

    #[test]
    fn schedule_tasks_have_the_shape_of_their_kind() {
        for spec in schedule_specs() {
            let task = &spec.task;
            match spec.kind {
                ScheduleKind::Once => {
                    assert!(task.repeat.is_none() && task.at > at(0));
                    assert_eq!(task.status, TaskStatus::Pending);
                }
                ScheduleKind::Every => assert!(matches!(task.repeat, Some(Repeat::Every { .. }))),
                ScheduleKind::Daily => assert!(matches!(task.repeat, Some(Repeat::Daily { .. }))),
                ScheduleKind::Due => {
                    assert!(task.at <= at(0) && !task.late);
                    assert_eq!(task.status, TaskStatus::Pending);
                }
                ScheduleKind::Late => {
                    assert!(task.late && task.at < at(0));
                }
                ScheduleKind::Running => {
                    assert_eq!(task.status, TaskStatus::Running);
                    assert!(task.deadline.is_some());
                }
                ScheduleKind::Proposed => assert_eq!(task.status, TaskStatus::Proposed),
                ScheduleKind::WorkerTimeout => {
                    assert!(matches!(task.kind, TaskKind::Worker { .. }));
                    assert!(task.timeout_secs.is_some() && task.deadline.is_some());
                }
            }
        }
    }
}
