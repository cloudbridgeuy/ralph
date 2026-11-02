use super::monitor::Monitor;
use crate::cursor_owner::{now_secs, write_file};
use crate::warn::warn_if_err;
use ralph_core::chat_status::RunningWorker;
use ralph_core::worker::{activity_path, sent_path, ActivityMark, HarnessId, Report};
use ralph_core::worker_check::CHECK_STATUS;
use ralph_core::worker_stall::{
    alert_text, auto_continue_log_path, auto_continued_path, log_line, stall_action, AutoContinue,
    StallAction, StallFacts, CONTINUE_TEXT, REASON_API_ERROR,
};
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::Path;

fn read_mark(store: &Path, id: &str) -> Option<ActivityMark> {
    std::fs::read_to_string(activity_path(store, id))
        .ok()
        .and_then(|text| ActivityMark::parse(&text))
}

fn read_record(store: &Path, id: &str) -> Option<AutoContinue> {
    std::fs::read_to_string(auto_continued_path(store, id))
        .ok()
        .and_then(|text| AutoContinue::parse(&text))
}

fn facts(store: &Path, id: &str, check_secs: u64) -> StallFacts {
    StallFacts {
        activity: read_mark(store, id),
        reported: !sent_path(store, id).exists(),
        record: read_record(store, id),
        now: now_secs(),
        check_secs,
    }
}

fn append_line(path: &Path, line: &str) -> io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{line}")
}

impl Monitor {
    pub(super) fn stall_tick(&self, running: &[RunningWorker]) {
        for worker in running {
            self.stall_step(worker);
        }
    }

    fn check_secs(&self) -> u64 {
        self.worker_check_secs.unwrap_or_default()
    }

    fn stall_step(&self, worker: &RunningWorker) {
        let id = &worker.id;
        let found = facts(&self.store_dir, id, self.check_secs());
        if found.reported && found.record.is_some() {
            std::fs::remove_file(auto_continued_path(&self.store_dir, id)).ok();
            return;
        }
        match stall_action(&found) {
            StallAction::None => {}
            StallAction::Continue => self.auto_continue_worker(worker),
            StallAction::Alert => self.alert_stalled(worker, found.record),
        }
    }

    fn auto_continue_worker(&self, worker: &RunningWorker) {
        let id = &worker.id;
        if stall_action(&facts(&self.store_dir, id, self.check_secs())) != StallAction::Continue {
            return;
        }
        let at = now_secs();
        let sent = self.workers.nudge(id, CONTINUE_TEXT);
        if let Err(e) = sent {
            crate::warn::warn(format!("auto-continuing worker {id}: {e}"));
            return;
        }
        let record = AutoContinue::new(at, REASON_API_ERROR);
        warn_if_err(
            write_file(&auto_continued_path(&self.store_dir, id), &record.render()),
            format!("recording the auto-continue of worker {id}"),
        );
        let line = log_line(at, id, worker.kind.as_str(), REASON_API_ERROR);
        warn_if_err(
            append_line(&auto_continue_log_path(&self.store_dir), &line),
            format!("logging the auto-continue of worker {id}"),
        );
    }

    fn alert_stalled(&self, worker: &RunningWorker, record: Option<AutoContinue>) {
        let id = &worker.id;
        let tmux = (worker.kind == HarnessId::Claude).then_some(worker.session.as_str());
        let report = Report {
            worker: id.clone(),
            at: chrono::Utc::now().to_rfc3339(),
            status: CHECK_STATUS.to_string(),
            text: alert_text(id, tmux),
        };
        warn_if_err(
            self.inbox.post(&report, 0),
            format!("posting check for worker {id}"),
        );
        if let Some(record) = record {
            warn_if_err(
                write_file(
                    &auto_continued_path(&self.store_dir, id),
                    &record.alerted().render(),
                ),
                format!("recording the stall alert of worker {id}"),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::worker::ActivityState;

    #[test]
    fn facts_read_the_activity_the_sent_marker_and_the_record_from_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path();
        write_file(&activity_path(store, "A"), "100 failed").unwrap();
        write_file(&sent_path(store, "A"), "").unwrap();
        let record = AutoContinue::new(90, REASON_API_ERROR);
        write_file(&auto_continued_path(store, "A"), &record.render()).unwrap();
        let found = facts(store, "A", 30);
        assert_eq!(
            found.activity,
            Some(ActivityMark {
                at: 100,
                state: ActivityState::Failed
            })
        );
        assert!(!found.reported);
        assert_eq!(found.record, Some(record));
        assert_eq!(stall_action(&found), StallAction::Alert);
    }

    #[test]
    fn facts_for_a_worker_that_reported_have_no_sent_marker() {
        let dir = tempfile::tempdir().unwrap();
        let found = facts(dir.path(), "A", 30);
        assert_eq!(found.activity, None);
        assert!(found.reported);
        assert_eq!(found.record, None);
    }

    #[test]
    fn append_line_appends_one_line_per_call() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.jsonl");
        append_line(&path, "a").unwrap();
        append_line(&path, "b").unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "a\nb\n");
    }
}
