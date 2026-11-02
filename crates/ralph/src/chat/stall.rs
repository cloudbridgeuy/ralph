use super::monitor::Monitor;
use crate::cursor_owner::{now_secs, write_file};
use crate::warn::warn_if_err;
use crate::watches::locked;
use ralph_core::chat_status::RunningWorker;
use ralph_core::worker::{activity_path, sent_path, ActivityMark, HarnessId, Report};
use ralph_core::worker_check::CHECK_STATUS;
use ralph_core::worker_stall::{
    after_send, alert_text, auto_continue_log_path, auto_continued_path, gate_send, log_line,
    report_secs, reported_since_go_ahead, stall_action, AutoContinue, ContinueStep, StallAction,
    StallFacts, CONTINUE_TEXT, REASON_API_ERROR,
};
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::Path;
use std::time::UNIX_EPOCH;

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

fn sent_secs(store: &Path, id: &str) -> Option<u64> {
    let modified = std::fs::metadata(sent_path(store, id))
        .and_then(|meta| meta.modified())
        .ok()?;
    modified
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|elapsed| elapsed.as_secs())
}

struct FactsInput<'a> {
    store: &'a Path,
    id: &'a str,
    last_report: Option<&'a str>,
    check_secs: u64,
}

fn facts(input: &FactsInput) -> StallFacts {
    let FactsInput {
        store,
        id,
        last_report,
        check_secs,
    } = *input;
    let sent_at = sent_secs(store, id);
    StallFacts {
        activity: read_mark(store, id),
        reported: reported_since_go_ahead(sent_at, last_report.and_then(report_secs)),
        record: read_record(store, id),
        sent_at,
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

    fn facts_of(&self, id: &str) -> StallFacts {
        let last_report = locked(&self.check).reports.get(id).cloned();
        facts(&FactsInput {
            store: &self.store_dir,
            id,
            last_report: last_report.as_deref(),
            check_secs: self.worker_check_secs.unwrap_or_default(),
        })
    }

    fn stall_step(&self, worker: &RunningWorker) {
        let id = &worker.id;
        let found = self.facts_of(id);
        if found.reported && found.record.is_some() {
            std::fs::remove_file(auto_continued_path(&self.store_dir, id)).ok();
            return;
        }
        match stall_action(&found) {
            StallAction::None => {}
            StallAction::Continue => self.auto_continue_worker(worker),
            StallAction::ResetRecord => {
                std::fs::remove_file(auto_continued_path(&self.store_dir, id)).ok();
            }
            StallAction::Alert => self.alert_stalled(worker, found.record),
        }
    }

    fn auto_continue_worker(&self, worker: &RunningWorker) {
        let id = &worker.id;
        if stall_action(&self.facts_of(id)) != StallAction::Continue {
            return;
        }
        let at = now_secs();
        if !self.record_continue(id, at) {
            return;
        }
        let sent = self.workers.nudge(id, CONTINUE_TEXT);
        if let Err(e) = &sent {
            crate::warn::warn(format!("auto-continuing worker {id}: {e}"));
        }
        self.run_steps(worker, at, after_send(sent.is_ok()));
    }

    fn record_continue(&self, id: &str, at: u64) -> bool {
        let written = write_file(
            &auto_continued_path(&self.store_dir, id),
            &AutoContinue::new(at, REASON_API_ERROR).render(),
        );
        let gate = {
            let mut unrecorded = locked(&self.unrecorded);
            let gate = gate_send(written.is_ok(), unrecorded.contains(id));
            if written.is_ok() {
                unrecorded.remove(id);
            } else {
                unrecorded.insert(id.to_string());
            }
            gate
        };
        if let (true, Err(e)) = (gate.warn, &written) {
            crate::warn::warn(format!(
                "recording the auto-continue of worker {id}, no continue sent: {e}"
            ));
        }
        gate.send
    }

    fn run_steps(&self, worker: &RunningWorker, at: u64, steps: &[ContinueStep]) {
        let id = &worker.id;
        let record_path = auto_continued_path(&self.store_dir, id);
        for step in steps {
            match step {
                ContinueStep::StampRecord => warn_if_err(
                    write_file(
                        &record_path,
                        &AutoContinue::new(now_secs(), REASON_API_ERROR).render(),
                    ),
                    format!("stamping the auto-continue record of worker {id}"),
                ),
                ContinueStep::RemoveRecord => {
                    std::fs::remove_file(&record_path).ok();
                }
                ContinueStep::Log => warn_if_err(
                    append_line(
                        &auto_continue_log_path(&self.store_dir),
                        &log_line(at, id, worker.kind.as_str(), REASON_API_ERROR),
                    ),
                    format!("logging the auto-continue of worker {id}"),
                ),
            }
        }
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

    fn input<'a>(store: &'a Path, last_report: Option<&'a str>) -> FactsInput<'a> {
        FactsInput {
            store,
            id: "A",
            last_report,
            check_secs: 30,
        }
    }

    #[test]
    fn a_report_after_the_go_ahead_makes_the_facts_reported() {
        let dir = tempfile::tempdir().unwrap();
        write_file(&sent_path(dir.path(), "A"), "").unwrap();
        let none = facts(&input(dir.path(), None));
        let old = facts(&input(dir.path(), Some("1999-01-01T00:00:00Z")));
        let new = facts(&input(dir.path(), Some("2999-01-01T00:00:00Z")));
        assert!(!none.reported);
        assert!(!old.reported);
        assert!(new.reported);
    }

    #[test]
    fn facts_read_the_activity_the_sent_marker_and_the_record_from_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path();
        write_file(&activity_path(store, "A"), "100 failed").unwrap();
        write_file(&sent_path(store, "A"), "").unwrap();
        std::fs::File::options()
            .write(true)
            .open(sent_path(store, "A"))
            .unwrap()
            .set_modified(UNIX_EPOCH + std::time::Duration::from_secs(50))
            .unwrap();
        let record = AutoContinue::new(90, REASON_API_ERROR);
        write_file(&auto_continued_path(store, "A"), &record.render()).unwrap();
        let found = facts(&input(store, None));
        assert_eq!(
            found.activity,
            Some(ActivityMark {
                at: 100,
                state: ActivityState::Failed
            })
        );
        assert!(!found.reported);
        assert_eq!(found.sent_at, Some(50));
        assert_eq!(found.record, Some(record));
        assert_eq!(stall_action(&found), StallAction::Alert);
    }

    #[test]
    fn facts_for_a_worker_that_reported_have_no_sent_marker() {
        let dir = tempfile::tempdir().unwrap();
        let found = facts(&input(dir.path(), None));
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
