use crate::chat::terminal::{poll_input, read_input, Input, POLL_MS};
use crate::schedule_store::ScheduleStore;
use crate::signal;
use crate::warn::warn_if_err;
use ralph_core::schedule::{approval_question, parse_answer, proposed, Answer, Task, TaskStatus};
use std::io::{self, Write};
use std::sync::atomic::Ordering;
use std::time::Duration;

fn read_answer() -> Option<String> {
    let interrupt = signal::interrupt_flag();
    interrupt.store(false, Ordering::SeqCst);
    loop {
        if signal::is_terminated() || interrupt.swap(false, Ordering::SeqCst) {
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

pub(super) fn approve_proposed(store: &ScheduleStore, default_timeout: Duration) {
    let mut out = io::stdout();
    approve_with(store, default_timeout, &mut read_answer, &mut out);
}

fn approve_with(
    store: &ScheduleStore,
    default_timeout: Duration,
    read: &mut impl FnMut() -> Option<String>,
    out: &mut impl Write,
) {
    for task in proposed(store.load_all()) {
        if decide(&task, default_timeout, read, out) {
            warn_if_err(
                store.save(&Task {
                    status: TaskStatus::Pending,
                    ..task.clone()
                }),
                format!("arming scheduled task {}", task.id),
            );
            say(out, &format!("Armed {}.", task.id));
        } else {
            store.remove(&task.id);
            say(out, &format!("Cancelled {}.", task.id));
        }
    }
}

fn decide(
    task: &Task,
    default_timeout: Duration,
    read: &mut impl FnMut() -> Option<String>,
    out: &mut impl Write,
) -> bool {
    loop {
        let now = chrono::Utc::now();
        let offset = *chrono::Local::now().offset();
        say_raw(out, &approval_question(task, now, offset, default_timeout));
        let Some(line) = read() else {
            say_raw(out, "\n");
            return false;
        };
        match parse_answer(&line) {
            Answer::Yes => return true,
            Answer::No => return false,
            Answer::Ask => {}
        }
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

    fn propose(store: &ScheduleStore, id: &str, minutes: i64) {
        store
            .save(&Task {
                id: id.to_string(),
                kind: TaskKind::Command,
                command: format!("echo {id}"),
                follow_up: None,
                at: Utc::now() + chrono::Duration::minutes(minutes),
                timeout_secs: None,
                status: TaskStatus::Proposed,
                late: false,
            })
            .unwrap();
    }

    fn run(store: &ScheduleStore, answers: &[Option<&str>]) -> String {
        let mut queue = answers.iter().map(|a| a.map(str::to_string));
        let mut out = Vec::new();
        approve_with(
            store,
            Duration::from_secs(60),
            &mut || queue.next().flatten(),
            &mut out,
        );
        String::from_utf8(out).unwrap()
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
        let out = run(&store, &[Some("YES\n"), Some("n\n")]);
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
        let out = run(&store, &[Some("go\n"), Some("\n"), Some("y\n")]);
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
        let out = run(&store, &[None]);
        assert!(statuses(&store).is_empty());
        assert!(out.contains("Cancelled a."));
    }

    #[test]
    fn armed_and_running_tasks_are_not_asked_about() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScheduleStore::new(dir.path().to_path_buf());
        propose(&store, "a", 5);
        run(&store, &[Some("y")]);
        let out = run(&store, &[]);
        assert!(out.is_empty());
        assert_eq!(
            statuses(&store),
            vec![("a".to_string(), TaskStatus::Pending)]
        );
    }
}
