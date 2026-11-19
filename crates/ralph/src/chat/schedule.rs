use crate::chat::terminal::{poll_input, read_input, Input, POLL_MS};
use crate::signal;
use ralph_core::cancel::CancelToken;
use ralph_core::schedule::{approval_question, parse_answer, proposed, Answer, Task, TaskStatus};
use ralph_core::schedule_command::{
    cancel_message, list_tasks, plan_cancel, CancelPlan, CancelStop, ScheduleCommand,
    SCHEDULE_USAGE,
};
use ralph_server::schedule_store::ScheduleStore;
use ralph_server::warn::warn_if_err;
use ralph_server::watches::Watches;
use ralph_server::workers::Workers;
use std::io::{self, Write};
use std::time::Duration;

pub(super) fn read_answer() -> Option<String> {
    let cancel = CancelToken::default();
    let _armed = signal::arm(&cancel);
    loop {
        if signal::is_terminated() || cancel.is_cancelled() {
            return None;
        }
        match poll_input(POLL_MS) {
            Input::Closed => return None,
            Input::Idle => {}
            Input::Ready => {
                let bytes = read_input();
                return (!bytes.is_empty()).then(|| String::from_utf8_lossy(&bytes).into_owned());
            }
        }
    }
}

pub(super) fn approve_proposed(store: &ScheduleStore, default_timeout: Duration) -> bool {
    let mut out = io::stdout();
    approve_with(store, default_timeout, &mut read_answer, &mut out)
}

fn approve_with(
    store: &ScheduleStore,
    default_timeout: Duration,
    read: &mut impl FnMut() -> Option<String>,
    out: &mut impl Write,
) -> bool {
    let tasks = proposed(store.load_all());
    for task in &tasks {
        if decide(task, default_timeout, read, out) {
            let armed = store.save(&Task {
                status: TaskStatus::Pending,
                ..task.clone()
            });
            match armed {
                Ok(()) => say(out, &format!("Armed {}.", task.id)),
                Err(error) => say(out, &format!("Could not arm {}: {error}", task.id)),
            }
        } else {
            store.remove(&task.id);
            say(out, &format!("Cancelled {}.", task.id));
        }
    }
    !tasks.is_empty()
}

fn decide(
    task: &Task,
    default_timeout: Duration,
    read: &mut impl FnMut() -> Option<String>,
    out: &mut impl Write,
) -> bool {
    let question = || {
        let now = chrono::Utc::now();
        let offset = *chrono::Local::now().offset();
        approval_question(task, now, offset, default_timeout)
    };
    ask_yes_no(question, read, out)
}

pub(super) fn ask_yes_no(
    question: impl Fn() -> String,
    read: &mut impl FnMut() -> Option<String>,
    out: &mut impl Write,
) -> bool {
    ask_answer(question, read, out).unwrap_or(false)
}

pub(super) fn ask_answer(
    question: impl Fn() -> String,
    read: &mut impl FnMut() -> Option<String>,
    out: &mut impl Write,
) -> Option<bool> {
    loop {
        say_raw(out, &question());
        let Some(line) = read() else {
            say_raw(out, "\n");
            return None;
        };
        match parse_answer(&line) {
            Answer::Yes => return Some(true),
            Answer::No => return Some(false),
            Answer::Ask => {}
        }
    }
}

pub(super) fn run(
    command: ScheduleCommand,
    store: &ScheduleStore,
    watches: &Watches,
    workers: &Workers,
) -> String {
    match command {
        ScheduleCommand::List => list_tasks(
            &store.load_all(),
            chrono::Utc::now(),
            *chrono::Local::now().offset(),
        ),
        ScheduleCommand::Cancel(id) => {
            let plan = plan_cancel(&id, &store.load_all());
            if let CancelPlan::Remove { id, stop } = &plan {
                store.remove(id);
                match stop {
                    Some(CancelStop::Watch(watch)) => {
                        watches.stop(watch);
                    }
                    Some(CancelStop::Worker(worker)) => warn_if_err(
                        workers.stop_harness(worker),
                        format!("stopping cancelled worker {worker}"),
                    ),
                    None => {}
                }
            }
            cancel_message(&plan)
        }
        ScheduleCommand::Usage => SCHEDULE_USAGE.to_string(),
    }
}

fn say(out: &mut impl Write, line: &str) {
    say_raw(out, &format!("{line}\n"));
}

fn say_raw(out: &mut impl Write, text: &str) {
    let _ = write!(out, "{text}");
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ralph_core::schedule::TaskKind;

    fn answers(lines: &[&str]) -> impl FnMut() -> Option<String> {
        let mut lines: Vec<String> = lines.iter().rev().map(|l| l.to_string()).collect();
        move || lines.pop()
    }

    #[test]
    fn y_confirms_and_n_declines() {
        let mut out = Vec::new();
        assert!(ask_yes_no(
            || "go? ".to_string(),
            &mut answers(&["y"]),
            &mut out
        ));
        assert!(!ask_yes_no(
            || "go? ".to_string(),
            &mut answers(&["n"]),
            &mut out
        ));
    }

    #[test]
    fn another_answer_asks_again() {
        let mut out = Vec::new();
        assert!(ask_yes_no(
            || "go? ".to_string(),
            &mut answers(&["maybe", "", "yes"]),
            &mut out
        ));
        assert_eq!(String::from_utf8(out).unwrap().matches("go? ").count(), 3);
    }

    #[test]
    fn a_closed_input_is_no_answer() {
        let mut out = Vec::new();
        assert_eq!(
            ask_answer(|| "go? ".to_string(), &mut answers(&[]), &mut out),
            None
        );
        assert_eq!(
            ask_answer(|| "go? ".to_string(), &mut answers(&["no"]), &mut out),
            Some(false)
        );
    }

    #[test]
    fn a_closed_input_declines() {
        let mut out = Vec::new();
        assert!(!ask_yes_no(
            || "go? ".to_string(),
            &mut answers(&[]),
            &mut out
        ));
    }

    fn propose(store: &ScheduleStore, id: &str, minutes: i64) {
        store
            .save(&Task {
                id: id.to_string(),
                kind: TaskKind::Command {
                    command: format!("echo {id}"),
                },
                follow_up: None,
                at: Utc::now() + chrono::Duration::minutes(minutes),
                timeout_secs: None,
                status: TaskStatus::Proposed,
                late: false,
                deadline: None,
            })
            .unwrap();
    }

    fn run(store: &ScheduleStore, answers: &[Option<&str>]) -> (bool, String) {
        let mut queue = answers.iter().map(|a| a.map(str::to_string));
        let mut out = Vec::new();
        let asked = approve_with(
            store,
            Duration::from_secs(60),
            &mut || queue.next().flatten(),
            &mut out,
        );
        (asked, String::from_utf8(out).unwrap())
    }

    fn statuses(store: &ScheduleStore) -> Vec<(String, TaskStatus)> {
        let mut tasks: Vec<_> = store
            .load_all()
            .into_iter()
            .map(|t| (t.id, t.status))
            .collect();
        tasks.sort_by(|a, b| a.0.cmp(&b.0));
        tasks
    }

    #[test]
    fn yes_arms_and_no_cancels_in_time_order() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScheduleStore::new(dir.path().to_path_buf());
        propose(&store, "first", 5);
        propose(&store, "second", 10);
        let (_, out) = run(&store, &[Some("YES\n"), Some("n\n")]);
        assert_eq!(
            statuses(&store),
            vec![("first".to_string(), TaskStatus::Pending)]
        );
        assert!(out.contains("Armed first."));
        assert!(out.contains("Cancelled second."));
        assert!(out.find("echo first").unwrap() < out.find("echo second").unwrap());
    }

    #[test]
    fn unclear_answers_repeat_the_same_question() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScheduleStore::new(dir.path().to_path_buf());
        propose(&store, "a", 5);
        let (_, out) = run(&store, &[Some("go\n"), Some("\n"), Some("y\n")]);
        assert_eq!(out.matches("Arm it? [y/n]").count(), 3);
        assert_eq!(
            statuses(&store),
            vec![("a".to_string(), TaskStatus::Pending)]
        );
    }

    #[test]
    fn interrupt_or_end_of_input_cancels_the_task() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScheduleStore::new(dir.path().to_path_buf());
        propose(&store, "a", 5);
        let (_, out) = run(&store, &[None]);
        assert!(statuses(&store).is_empty());
        assert!(out.contains("Cancelled a."));
    }

    #[test]
    fn armed_and_running_tasks_are_not_asked_about() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScheduleStore::new(dir.path().to_path_buf());
        propose(&store, "a", 5);
        assert!(run(&store, &[Some("y")]).0);
        let (asked, out) = run(&store, &[]);
        assert!(!asked);
        assert!(out.is_empty());
        assert_eq!(
            statuses(&store),
            vec![("a".to_string(), TaskStatus::Pending)]
        );
    }

    #[test]
    fn an_empty_store_asks_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScheduleStore::new(dir.path().to_path_buf());
        assert_eq!(run(&store, &[]), (false, String::new()));
    }

    #[test]
    fn a_failed_save_is_reported_instead_of_armed() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScheduleStore::new(dir.path().to_path_buf());
        propose(&store, "a", 5);
        let tmp = dir
            .path()
            .join(format!(".{}", ralph_core::watch_store::file_name("a")));
        std::fs::create_dir(&tmp).unwrap();
        let (_, out) = run(&store, &[Some("y")]);
        assert!(out.contains("Could not arm a:"), "{out}");
        assert!(!out.contains("Armed a."), "{out}");
        assert_eq!(
            statuses(&store),
            vec![("a".to_string(), TaskStatus::Proposed)]
        );
    }
}
