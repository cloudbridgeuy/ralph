use super::{clear_marker, Workers};
use crate::cursor_owner::write_file;
use ralph_core::worker::{close_handoff, Handoff, HandoffFiles, PermissionAnswer, HANDOFF_POLL};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Instant;

impl Workers {
    pub(super) fn answer_permission(
        &self,
        id: &str,
        request_id: &str,
        text: &str,
    ) -> io::Result<String> {
        let HandoffFiles { answer, error, .. } = self.headless_files(id).ok_or_else(|| {
            io::Error::other(format!(
                "worker {id} is not a running Cursor, Codex or opencode worker"
            ))
        })?;
        let body = serde_json::to_string(&PermissionAnswer {
            request_id: request_id.to_string(),
            option_id: text.to_string(),
        })?;
        fs::create_dir_all(answer.parent().unwrap_or(&self.store_dir))?;
        let started = Instant::now();
        let _lock = lock_reply(&reply_lock_path(&self.store_dir, id), id, started)?;
        clear_marker(&error)?;
        let staged = reply_staging_path(&self.store_dir, id);
        write_file(&staged, &body)?;
        fs::rename(&staged, &answer).inspect_err(|_| {
            fs::remove_file(&staged).ok();
        })?;
        loop {
            match close_handoff(answer.exists(), started.elapsed()) {
                Handoff::Done => break,
                Handoff::TimedOut => {
                    clear_marker(&answer)?;
                    return Err(io::Error::other(
                        "ralph chat did not pick up the permission answer in time",
                    ));
                }
                Handoff::Pending | Handoff::Failed(_) => thread::sleep(HANDOFF_POLL),
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
    use ralph_core::worker::{blocked_path, HANDOFF_TIMEOUT};
    use std::time::Duration;

    #[test]
    fn a_permission_answer_carries_its_request_id_to_the_host() {
        let dir = tempfile::tempdir().unwrap();
        let workers = workers_in(dir.path());
        let files = HandoffFiles::opencode(dir.path(), "A");
        write_file(&files.owner, "").unwrap();
        write_file(&blocked_path(dir.path(), "A"), "").unwrap();
        let (answer, error) = (files.answer.clone(), files.error.clone());
        let host = thread::spawn(move || {
            let mut seen = Vec::new();
            while seen.len() < 2 {
                if let Ok(text) = fs::read_to_string(&answer) {
                    let decoded = PermissionAnswer::decode(&text).unwrap();
                    if decoded.request_id.is_empty() {
                        write_file(&error, "pass request. pending: per_1").unwrap();
                    }
                    seen.push(decoded);
                    fs::remove_file(&answer).unwrap();
                }
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
        let (answer, error) = (files.answer.clone(), files.error.clone());
        let (stale_written, stale_written_rx) = std::sync::mpsc::channel();
        let host = thread::spawn(move || {
            let mut handled = 0;
            while handled < 2 {
                if let Ok(text) = fs::read_to_string(&answer) {
                    if PermissionAnswer::decode(&text).unwrap().request_id == "stale" {
                        write_file(&error, "stale request stale").unwrap();
                        stale_written.send(()).unwrap();
                        thread::sleep(Duration::from_millis(300));
                    }
                    fs::remove_file(&answer).unwrap();
                    handled += 1;
                }
                thread::sleep(Duration::from_millis(5));
            }
        });
        let stale = {
            let workers = workers.clone();
            thread::spawn(move || workers.reply("A", "once", false, Some("stale")))
        };
        stale_written_rx.recv().unwrap();
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
        let (store, answer) = (dir.path().to_path_buf(), files.answer.clone());
        let host = thread::spawn(move || loop {
            let ids = crate::cursor_owner::request_ids(&store, "opencode-answer");
            if ids.contains(&"A".to_string()) && fs::remove_file(&answer).is_ok() {
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
}
