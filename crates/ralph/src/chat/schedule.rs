use crate::chat::editor::open_in_editor;
use crate::chat::terminal::{poll_input, read_input, Input, POLL_MS};
use crate::signal;
use ralph_core::cancel::CancelToken;
use ralph_core::schedule::{approval_question, proposed, Task, TaskStatus};
use ralph_core::schedule_command::{
    cancel_message, list_tasks, plan_cancel, CancelPlan, CancelStop, ScheduleCommand,
    SCHEDULE_USAGE,
};
use ralph_server::chat::ask::{yes_no, Ask};
use ralph_server::schedule_store::ScheduleStore;
use ralph_server::warn::warn_if_err;
use ralph_server::watches::Watches;
use ralph_server::workers::Workers;
use std::io::{self, Write};
use std::path::Path;
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

pub(super) struct Keyboard<'a> {
    pub editor: &'a [String],
}

impl Ask for Keyboard<'_> {
    fn answer(&mut self, prompt: &str) -> Option<String> {
        let mut out = io::stdout();
        say_raw(&mut out, prompt);
        let line = read_answer();
        if line.is_none() {
            say_raw(&mut out, "\n");
        }
        line
    }

    fn edit(&mut self, path: &Path) -> io::Result<()> {
        open_in_editor(self.editor, path)
    }
}

pub(super) fn approve_proposed(store: &ScheduleStore, default_timeout: Duration) -> bool {
    let mut out = io::stdout();
    let mut keyboard = Keyboard { editor: &[] };
    approve_with(store, default_timeout, &mut keyboard, &mut out)
}

fn approve_with(
    store: &ScheduleStore,
    default_timeout: Duration,
    ask: &mut dyn Ask,
    out: &mut impl Write,
) -> bool {
    let tasks = proposed(store.load_all());
    for task in &tasks {
        if decide(task, default_timeout, ask) {
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

fn decide(task: &Task, default_timeout: Duration, ask: &mut dyn Ask) -> bool {
    let question = || {
        let now = chrono::Utc::now();
        let offset = *chrono::Local::now().offset();
        approval_question(task, now, offset, default_timeout)
    };
    yes_no(ask, question).unwrap_or(false)
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

    struct Scripted {
        queue: std::vec::IntoIter<Option<String>>,
        prompts: String,
    }

    impl Ask for Scripted {
        fn answer(&mut self, prompt: &str) -> Option<String> {
            self.prompts.push_str(prompt);
            self.queue.next().flatten()
        }

        fn edit(&mut self, _path: &Path) -> io::Result<()> {
            Ok(())
        }
    }

    fn run(store: &ScheduleStore, answers: &[Option<&str>]) -> (bool, String) {
        let mut ask = Scripted {
            queue: answers
                .iter()
                .map(|a| a.map(str::to_string))
                .collect::<Vec<_>>()
                .into_iter(),
            prompts: String::new(),
        };
        let mut out = Vec::new();
        let asked = approve_with(store, Duration::from_secs(60), &mut ask, &mut out);
        (asked, ask.prompts + &String::from_utf8(out).unwrap())
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
