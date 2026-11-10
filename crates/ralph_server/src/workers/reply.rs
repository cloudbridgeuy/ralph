use super::{clear_marker, Workers};
use crate::cursor_owner::write_file;
use ralph_core::worker::{
    capabilities, close_handoff, describe, Feature, Handoff, HandoffFiles, PermissionAnswer,
    Support, HANDOFF_POLL,
};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

impl Workers {
    pub(super) fn answer_permission(
        &self,
        id: &str,
        request_id: &str,
        text: &str,
    ) -> io::Result<String> {
        let (kind, files) = self.headless_files(id).ok_or_else(|| {
            io::Error::other(format!(
                "worker {id} is not a running Cursor, Codex, claude-headless or opencode worker"
            ))
        })?;
        let caps = capabilities(kind);
        if caps.support(Feature::PermAsk) == Support::None {
            return Err(io::Error::other(describe(&caps, Feature::PermAsk)));
        }
        let HandoffFiles {
            answer,
            error,
            claim,
            ..
        } = files;
        let body = serde_json::to_string(&PermissionAnswer {
            request_id: request_id.to_string(),
            option_id: text.to_string(),
        })?;
        fs::create_dir_all(answer.parent().unwrap_or(&self.store_dir))?;
        let _lock = lock_reply(&reply_lock_path(&self.store_dir, id), id, Instant::now())?;
        if !settle(self.handoff_timeout, || claim.exists()) {
            return Err(io::Error::other(format!(
                "ralph chat is still handling an earlier permission answer for worker {id}"
            )));
        }
        clear_marker(&error)?;
        let staged = reply_staging_path(&self.store_dir, id);
        write_file(&staged, &body)?;
        fs::rename(&staged, &answer).inspect_err(|_| {
            fs::remove_file(&staged).ok();
        })?;
        if !settle(self.handoff_timeout, || answer.exists() || claim.exists()) {
            let removed = match fs::remove_file(&answer) {
                Ok(()) => true,
                Err(error) if error.kind() == io::ErrorKind::NotFound => false,
                Err(error) => return Err(error),
            };
            match timeout_outcome(removed, claim.exists()) {
                TimeoutOutcome::NotPickedUp => {
                    return Err(io::Error::other(
                        "ralph chat did not pick up the permission answer in time",
                    ));
                }
                TimeoutOutcome::OutcomeUnknown => {
                    return Err(io::Error::other(format!(
                        "ralph chat took the permission answer for worker {id} but did not finish in time; outcome unknown"
                    )));
                }
                TimeoutOutcome::Finished => {}
            }
        }
        match fs::read_to_string(&error) {
            Ok(message) => {
                clear_marker(&error)?;
                Err(io::Error::other(message))
            }
            Err(_) => Ok(format!("Answered the permission request of worker {id}.")),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Wait {
    Pending,
    Done,
    TimedOut,
}

fn handoff_wait(pending: bool, elapsed: Duration, timeout: Duration) -> Wait {
    if !pending {
        Wait::Done
    } else if elapsed >= timeout {
        Wait::TimedOut
    } else {
        Wait::Pending
    }
}

#[derive(Debug, PartialEq, Eq)]
enum TimeoutOutcome {
    NotPickedUp,
    OutcomeUnknown,
    Finished,
}

fn timeout_outcome(removed: bool, claimed: bool) -> TimeoutOutcome {
    if removed {
        TimeoutOutcome::NotPickedUp
    } else if claimed {
        TimeoutOutcome::OutcomeUnknown
    } else {
        TimeoutOutcome::Finished
    }
}

fn settle(timeout: Duration, pending: impl Fn() -> bool) -> bool {
    let started = Instant::now();
    loop {
        match handoff_wait(pending(), started.elapsed(), timeout) {
            Wait::Done => return true,
            Wait::TimedOut => return false,
            Wait::Pending => thread::sleep(poll_interval(timeout)),
        }
    }
}

fn poll_interval(timeout: Duration) -> Duration {
    (timeout / 10).min(HANDOFF_POLL)
}

fn reply_lock_path(store_dir: &Path, id: &str) -> PathBuf {
    store_dir.join(format!(".answer-{id}.lock"))
}

fn reply_staging_path(store_dir: &Path, id: &str) -> PathBuf {
    store_dir.join(format!(".answer-{id}"))
}

fn lock_reply(lock: &Path, id: &str, started: Instant) -> io::Result<fs::File> {
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock)?;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(fs::TryLockError::WouldBlock) => {}
            Err(fs::TryLockError::Error(error)) => return Err(error),
        }
        if close_handoff(true, started.elapsed()) == Handoff::TimedOut {
            return Err(io::Error::other(format!(
                "another reply to worker {id} is still in progress"
            )));
        }
        thread::sleep(HANDOFF_POLL);
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::workers_in;
    use super::*;
    use crate::cursor_owner::take_answer;
    use ralph_core::worker::{blocked_path, HANDOFF_TIMEOUT};

    #[test]
    fn a_permission_answer_carries_its_request_id_to_the_host() {
        let dir = tempfile::tempdir().unwrap();
        let workers = workers_in(dir.path());
        let files = HandoffFiles::opencode(dir.path(), "A");
        write_file(&files.owner, "").unwrap();
        write_file(&blocked_path(dir.path(), "A"), "").unwrap();
        let host_files = files.clone();
        let host = thread::spawn(move || {
            let mut seen = Vec::new();
            while seen.len() < 2 {
                take_answer(&host_files, |text| {
                    let decoded = PermissionAnswer::decode(text).unwrap();
                    let empty = decoded.request_id.is_empty();
                    seen.push(decoded);
                    if empty {
                        return Err("pass request. pending: per_1".to_string());
                    }
                    Ok(())
                });
                thread::sleep(Duration::from_millis(10));
            }
            seen
        });
        let unnamed = workers.reply("A", "once", false, None).unwrap_err();
        assert_eq!(unnamed.to_string(), "pass request. pending: per_1");
        assert_eq!(
            workers.reply("A", "once", false, Some("per_1")).unwrap(),
            "Answered the permission request of worker A."
        );
        let seen = host.join().unwrap();
        assert_eq!(seen[1].request_id, "per_1");
        assert_eq!(seen[1].option_id, "once");
    }

    #[test]
    fn concurrent_replies_to_one_worker_each_get_their_own_result() {
        let dir = tempfile::tempdir().unwrap();
        let workers = workers_in(dir.path());
        let files = HandoffFiles::opencode(dir.path(), "A");
        write_file(&files.owner, "").unwrap();
        write_file(&blocked_path(dir.path(), "A"), "").unwrap();
        let host_files = files.clone();
        let (stale_taken, stale_taken_rx) = std::sync::mpsc::channel();
        let host = thread::spawn(move || {
            let mut handled = 0;
            while handled < 2 {
                take_answer(&host_files, |text| {
                    handled += 1;
                    if PermissionAnswer::decode(text).unwrap().request_id == "stale" {
                        stale_taken.send(()).unwrap();
                        thread::sleep(Duration::from_millis(300));
                        return Err("stale request stale".to_string());
                    }
                    Ok(())
                });
                thread::sleep(Duration::from_millis(5));
            }
        });
        let stale = {
            let workers = workers.clone();
            thread::spawn(move || workers.reply("A", "once", false, Some("stale")))
        };
        stale_taken_rx.recv().unwrap();
        let valid = {
            let workers = workers.clone();
            thread::spawn(move || workers.reply("A", "once", false, Some("valid")))
        };
        let stale = stale.join().unwrap();
        let valid = valid.join().unwrap();
        host.join().unwrap();
        assert_eq!(stale.unwrap_err().to_string(), "stale request stale");
        assert_eq!(
            valid.unwrap(),
            "Answered the permission request of worker A."
        );
    }

    #[test]
    fn a_reply_leaves_only_worker_ids_in_the_answer_folder() {
        let dir = tempfile::tempdir().unwrap();
        let workers = workers_in(dir.path());
        let files = HandoffFiles::opencode(dir.path(), "A");
        write_file(&files.owner, "").unwrap();
        write_file(&blocked_path(dir.path(), "A"), "").unwrap();
        let store = dir.path().to_path_buf();
        let host = thread::spawn(move || loop {
            let ids = crate::cursor_owner::request_ids(&store, "opencode-answer");
            if ids.contains(&"A".to_string()) {
                take_answer(&files, |_| Ok(()));
                return ids;
            }
            thread::sleep(Duration::from_millis(2));
        });
        workers.reply("A", "once", false, Some("per_1")).unwrap();
        assert_eq!(host.join().unwrap(), ["A"]);
        assert!(crate::cursor_owner::request_ids(dir.path(), "opencode-answer").is_empty());
    }

    #[test]
    fn the_reply_lock_and_staging_files_sit_in_the_store_root() {
        let store = Path::new("/s");
        assert_eq!(reply_lock_path(store, "A"), Path::new("/s/.answer-A.lock"));
        assert_eq!(reply_staging_path(store, "A"), Path::new("/s/.answer-A"));
    }

    #[test]
    fn a_held_reply_lock_times_out_a_second_reply() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("A.lock");
        let held = lock_reply(&lock, "A", Instant::now()).unwrap();
        let waited = Instant::now() - HANDOFF_TIMEOUT;
        let error = lock_reply(&lock, "A", waited).unwrap_err();
        assert_eq!(
            error.to_string(),
            "another reply to worker A is still in progress"
        );
        drop(held);
        assert!(lock_reply(&lock, "A", Instant::now()).is_ok());
    }

    fn lock_is_held(dir: &Path) -> bool {
        let lock = reply_lock_path(dir, "A");
        fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(lock)
            .is_ok_and(|file| matches!(file.try_lock(), Err(fs::TryLockError::WouldBlock)))
    }

    #[test]
    fn a_late_host_completion_cannot_consume_the_next_reply() {
        let dir = tempfile::tempdir().unwrap();
        let workers = Workers {
            handoff_timeout: Duration::from_millis(400),
            ..workers_in(dir.path())
        };
        let files = HandoffFiles::opencode(dir.path(), "A");
        write_file(&files.owner, "").unwrap();
        write_file(&blocked_path(dir.path(), "A"), "").unwrap();
        let (taken, taken_rx) = std::sync::mpsc::channel();
        let (release, release_rx) = std::sync::mpsc::channel();
        let host_files = files.clone();
        let host = thread::spawn(move || {
            let mut seen = Vec::new();
            while seen.len() < 2 {
                take_answer(&host_files, |text| {
                    seen.push(PermissionAnswer::decode(text).unwrap().request_id);
                    if seen.len() == 1 {
                        taken.send(()).unwrap();
                        release_rx.recv().unwrap();
                        return Err("stale request R1".to_string());
                    }
                    Ok(())
                });
                thread::sleep(Duration::from_millis(2));
            }
            seen
        });
        let first = {
            let workers = workers.clone();
            thread::spawn(move || workers.reply("A", "once", false, Some("R1")))
        };
        taken_rx.recv().unwrap();
        assert_eq!(
            first.join().unwrap().unwrap_err().to_string(),
            "ralph chat took the permission answer for worker A but did not finish in time; outcome unknown"
        );
        let second = {
            let workers = workers.clone();
            thread::spawn(move || workers.reply("A", "once", false, Some("R2")))
        };
        while !lock_is_held(dir.path()) {
            thread::sleep(Duration::from_millis(1));
        }
        thread::sleep(Duration::from_millis(50));
        release.send(()).unwrap();
        assert_eq!(
            second.join().unwrap().map_err(|error| error.to_string()),
            Ok("Answered the permission request of worker A.".to_string())
        );
        assert_eq!(host.join().unwrap(), ["R1", "R2"]);
        assert!(!files.claim.exists());
    }

    #[test]
    fn an_unclaimed_answer_is_withdrawn_when_the_handoff_times_out() {
        let dir = tempfile::tempdir().unwrap();
        let workers = Workers {
            handoff_timeout: Duration::from_millis(100),
            ..workers_in(dir.path())
        };
        let files = HandoffFiles::opencode(dir.path(), "A");
        write_file(&files.owner, "").unwrap();
        let error = workers
            .reply("A", "once", false, Some("per_1"))
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "ralph chat did not pick up the permission answer in time"
        );
        assert!(!files.answer.exists());
    }

    #[test]
    fn a_reply_waits_for_an_earlier_claim_before_it_clears_the_error() {
        let dir = tempfile::tempdir().unwrap();
        let workers = Workers {
            handoff_timeout: Duration::from_millis(100),
            ..workers_in(dir.path())
        };
        let files = HandoffFiles::opencode(dir.path(), "A");
        write_file(&files.owner, "").unwrap();
        write_file(&files.claim, "{}").unwrap();
        write_file(&files.error, "late error").unwrap();
        let error = workers
            .reply("A", "once", false, Some("per_1"))
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "ralph chat is still handling an earlier permission answer for worker A"
        );
        assert!(!files.answer.exists());
        assert_eq!(fs::read_to_string(&files.error).unwrap(), "late error");
    }

    #[test]
    fn no_answer_scanner_lists_a_claimed_answer() {
        let dir = tempfile::tempdir().unwrap();
        for files in [
            HandoffFiles::cursor(dir.path(), "A"),
            HandoffFiles::codex(dir.path(), "A"),
            HandoffFiles::opencode(dir.path(), "A"),
        ] {
            write_file(&files.claim, "{}").unwrap();
        }
        for answer_dir in ["cursor-answer", "codex-answer", "opencode-answer"] {
            assert!(crate::cursor_owner::request_ids(dir.path(), answer_dir).is_empty());
            assert!(crate::opencode_worker::entry_ids(dir.path(), answer_dir).is_empty());
        }
    }

    #[test]
    fn the_claim_never_collides_with_the_lock_or_staging_files() {
        let store = Path::new("/s");
        let claim = HandoffFiles::cursor(store, "A").claim;
        assert_ne!(claim, reply_lock_path(store, "A"));
        assert_ne!(claim, reply_staging_path(store, "A"));
    }

    #[test]
    fn a_handoff_wait_finishes_when_nothing_is_pending_and_times_out_at_the_limit() {
        let limit = Duration::from_millis(200);
        assert_eq!(handoff_wait(false, limit * 2, limit), Wait::Done);
        assert_eq!(handoff_wait(true, limit / 2, limit), Wait::Pending);
        assert_eq!(handoff_wait(true, limit, limit), Wait::TimedOut);
    }

    #[test]
    fn a_timeout_is_unknown_only_when_the_answer_was_taken_and_not_finished() {
        assert_eq!(timeout_outcome(true, false), TimeoutOutcome::NotPickedUp);
        assert_eq!(timeout_outcome(true, true), TimeoutOutcome::NotPickedUp);
        assert_eq!(timeout_outcome(false, true), TimeoutOutcome::OutcomeUnknown);
        assert_eq!(timeout_outcome(false, false), TimeoutOutcome::Finished);
    }

    #[test]
    fn the_poll_interval_shrinks_with_a_short_timeout_and_caps_at_the_default() {
        assert_eq!(
            poll_interval(Duration::from_millis(200)),
            Duration::from_millis(20)
        );
        assert_eq!(poll_interval(Duration::from_secs(30)), HANDOFF_POLL);
    }
}
