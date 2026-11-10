use chrono::{DateTime, Utc};
use ralph_core::watch_command::{stop_message, StopResult};
use ralph_core::watch_request::{
    already_queued_message, plan_queued, start_failure_report, WatchRequest, WATCH_REQUESTS_DIR,
};
use ralph_core::watch_store::file_name;
use ralph_core::watch_tools::{
    dequeued_message, format_rows, merge_rows, plan_stop, skipped_message, stop_report, StopStep,
    StopTarget,
};
use ralph_core::worker::Report;
use ralph_server::warn::{warn, warn_if_err};
use ralph_server::watches::Watches;
use ralph_server::worker_mcp::Inbox;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

fn requests_dir(store_dir: &Path) -> PathBuf {
    store_dir.join(WATCH_REQUESTS_DIR)
}

pub fn queue(store_dir: &Path, request: &WatchRequest) -> io::Result<()> {
    let dir = requests_dir(store_dir);
    fs::create_dir_all(&dir)?;
    let name = request.file_name();
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    fs::write(&tmp, serde_json::to_vec(request)?)?;
    let claimed = fs::hard_link(&tmp, dir.join(&name));
    warn_if_err(fs::remove_file(&tmp), format!("removing {}", tmp.display()));
    claimed.map_err(|e| match e.kind() {
        io::ErrorKind::AlreadyExists => io::Error::new(
            io::ErrorKind::AlreadyExists,
            already_queued_message(&request.id),
        ),
        _ => e,
    })
}

pub fn take_all(store_dir: &Path) -> Vec<WatchRequest> {
    let Ok(entries) = fs::read_dir(requests_dir(store_dir)) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| is_request(path))
        .collect();
    paths.sort();
    paths.iter().filter_map(|path| take(path)).collect()
}

pub fn list_queued(store_dir: &Path) -> Vec<WatchRequest> {
    let Ok(entries) = fs::read_dir(requests_dir(store_dir)) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| is_request(path))
        .collect();
    paths.sort();
    paths
        .iter()
        .filter_map(|path| serde_json::from_slice(&fs::read(path).ok()?).ok())
        .collect()
}

pub fn list_text(watches: &Watches, store_dir: &Path, now: DateTime<Utc>) -> String {
    format_rows(&merge_rows(
        &list_queued(store_dir),
        &watches.list(),
        &watches.saved(),
        now,
    ))
}

pub fn remove_queued(store_dir: &Path, id: &str) -> io::Result<()> {
    let claimed = claim(&requests_dir(store_dir).join(file_name(id)))?;
    warn_if_err(
        fs::remove_file(&claimed),
        format!("removing {}", claimed.display()),
    );
    Ok(())
}

/// Moves a request file to a private name. Only the caller whose rename
/// succeeds owns the request; the other side sees `NotFound`.
fn claim(path: &Path) -> io::Result<PathBuf> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let claimed = path.with_file_name(format!(".{name}.claimed"));
    fs::rename(path, &claimed)?;
    Ok(claimed)
}

pub fn stop_text(
    watches: &Watches,
    store_dir: &Path,
    target: &StopTarget,
    now: DateTime<Utc>,
) -> String {
    let _intake = watches.intake();
    let rows = merge_rows(
        &list_queued(store_dir),
        &watches.list(),
        &watches.saved(),
        now,
    );
    let lines: Vec<String> = plan_stop(target, &rows)
        .into_iter()
        .map(|step| match step {
            StopStep::Stop(id) => stop_message(&id, watches.stop(&id)),
            StopStep::Dequeue(id) => match remove_queued(store_dir, &id) {
                Ok(()) => dequeued_message(&id),
                Err(e) => format!("Could not remove queued watch {id}: {e}"),
            },
            StopStep::Skip { id, reason } => skipped_message(&id, reason),
            StopStep::Unknown(id) => stop_message(&id, StopResult::Unknown),
        })
        .collect();
    stop_report(&lines)
}

fn is_request(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "json")
        && path
            .file_name()
            .is_some_and(|name| !name.to_string_lossy().starts_with('.'))
}

fn take(path: &Path) -> Option<WatchRequest> {
    let claimed = claim(path).ok()?;
    let parsed = fs::read(&claimed)
        .map_err(|e| e.to_string())
        .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|e| e.to_string()));
    warn_if_err(
        fs::remove_file(&claimed),
        format!("removing {}", claimed.display()),
    );
    match parsed {
        Ok(request) => Some(request),
        Err(e) => {
            warn(format!(
                "discarding unreadable watch request {}: {e}",
                path.display()
            ));
            None
        }
    }
}

fn post_failure(inbox: &Inbox, report: &Report) {
    warn_if_err(
        inbox.post(report, 0),
        format!("posting watch {}", report.worker),
    );
}

pub fn start_queued(watches: &Watches, inbox: &Inbox, store_dir: &Path) {
    let _intake = watches.intake();
    let now = chrono::Utc::now().to_rfc3339();
    let plan = plan_queued(take_all(store_dir), &watches.used_ids(), &now);
    for report in &plan.rejected {
        post_failure(inbox, report);
    }
    for spec in plan.start {
        let id = spec.id.clone();
        if let Err(error) = watches.start(spec) {
            post_failure(inbox, &start_failure_report(&id, &error.to_string(), &now));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::watch::WatchKind;
    use ralph_core::watch_store::{SavedWatch, WATCHES_DIR};
    use ralph_server::watch_store::WatchStore;

    fn request(id: &str) -> WatchRequest {
        WatchRequest {
            id: id.to_string(),
            command: "true".to_string(),
            follow_up: "f".to_string(),
            persistent: false,
            timeout_secs: None,
        }
    }

    #[test]
    fn queued_requests_are_taken_once_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        queue(dir.path(), &request("a")).unwrap();
        queue(dir.path(), &request("b")).unwrap();
        let mut taken = take_all(dir.path());
        taken.sort_by(|x, y| x.id.cmp(&y.id));
        assert_eq!(taken, [request("a"), request("b")]);
        assert!(take_all(dir.path()).is_empty());
    }

    #[test]
    fn a_second_queue_of_the_same_id_errors_and_keeps_one_file() {
        let dir = tempfile::tempdir().unwrap();
        queue(dir.path(), &request("watch:a")).unwrap();
        let error = queue(dir.path(), &request("watch:a")).unwrap_err();
        assert_eq!(error.to_string(), "watch `watch:a` is already queued");
        let names: Vec<_> = fs::read_dir(requests_dir(dir.path()))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1);
        assert_eq!(take_all(dir.path()), [request("watch:a")]);
    }

    #[test]
    fn an_id_can_be_queued_again_after_it_was_taken() {
        let dir = tempfile::tempdir().unwrap();
        queue(dir.path(), &request("a")).unwrap();
        take_all(dir.path());
        queue(dir.path(), &request("a")).unwrap();
    }

    #[test]
    fn unreadable_and_temporary_files_are_not_requests() {
        let dir = tempfile::tempdir().unwrap();
        let requests = requests_dir(dir.path());
        fs::create_dir_all(&requests).unwrap();
        fs::write(requests.join("bad.json"), "{").unwrap();
        fs::write(requests.join(".half.json"), "{").unwrap();
        assert!(take_all(dir.path()).is_empty());
        assert!(!requests.join("bad.json").exists());
        assert!(requests.join(".half.json").exists());
    }

    #[test]
    fn listing_queued_requests_leaves_the_files() {
        let dir = tempfile::tempdir().unwrap();
        queue(dir.path(), &request("a")).unwrap();
        assert_eq!(list_queued(dir.path()), [request("a")]);
        assert_eq!(take_all(dir.path()), [request("a")]);
    }

    #[test]
    fn the_list_text_names_a_queued_request() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = std::sync::Arc::new(Inbox::new(dir.path().join("inbox")));
        let watches = Watches::new(inbox, dir.path(), std::time::Duration::from_secs(60));
        assert_eq!(
            list_text(&watches, dir.path(), Utc::now()),
            "No active watches."
        );
        queue(dir.path(), &request("watch:a")).unwrap();
        assert_eq!(
            list_text(&watches, dir.path(), Utc::now()),
            "watch:a queued normal - default - true"
        );
    }

    #[test]
    fn stopping_a_queued_request_removes_its_file_and_names_unknown_ids() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = std::sync::Arc::new(Inbox::new(dir.path().join("inbox")));
        let watches = Watches::new(inbox, dir.path(), std::time::Duration::from_secs(60));
        queue(dir.path(), &request("watch:a")).unwrap();
        let target = StopTarget::Ids(vec!["watch:a".to_string(), "watch:b".to_string()]);
        let text = stop_text(&watches, dir.path(), &target, Utc::now());
        assert_eq!(
            text,
            "Removed queued watch watch:a before it started.\nNo watch watch:b. /watch list shows active watches."
        );
        assert!(list_queued(dir.path()).is_empty());
        assert_eq!(
            stop_text(&watches, dir.path(), &StopTarget::All, Utc::now()),
            "No active watches."
        );
    }

    fn saved(id: &str, kind: WatchKind) -> SavedWatch {
        SavedWatch {
            id: id.to_string(),
            command: "sleep 99".to_string(),
            follow_up: Some("f".to_string()),
            kind,
            timeout_secs: None,
            started_at: Utc::now(),
            timeout_deadline: None,
            wake_deadline: None,
        }
    }

    fn save(dir: &Path, record: &SavedWatch) {
        WatchStore::new(dir.join(WATCHES_DIR)).save(record).unwrap();
    }

    fn watches_in(dir: &Path) -> Watches {
        let inbox = std::sync::Arc::new(Inbox::new(dir.join("inbox")));
        Watches::new(inbox, dir, std::time::Duration::from_secs(60))
    }

    #[test]
    fn stopping_a_saved_persistent_watch_removes_its_record() {
        let dir = tempfile::tempdir().unwrap();
        let watches = watches_in(dir.path());
        save(dir.path(), &saved("watch:p", WatchKind::Persistent));
        let target = StopTarget::Ids(vec!["watch:p".to_string()]);
        let text = stop_text(&watches, dir.path(), &target, Utc::now());
        assert_eq!(text, stop_message("watch:p", StopResult::RemovedSaved));
        assert!(watches.saved().is_empty());
    }

    #[test]
    fn stopping_all_handles_each_state_and_names_what_it_skips() {
        let dir = tempfile::tempdir().unwrap();
        let watches = watches_in(dir.path());
        save(dir.path(), &saved("watch:p", WatchKind::Persistent));
        save(dir.path(), &saved("watch:send-it-1", WatchKind::Normal));
        queue(dir.path(), &request("watch:q")).unwrap();
        let text = stop_text(&watches, dir.path(), &StopTarget::All, Utc::now());
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "{text}");
        assert!(lines.contains(&"Removed queued watch watch:q before it started."));
        assert!(lines.contains(&stop_message("watch:p", StopResult::RemovedSaved).as_str()));
        assert!(lines
            .iter()
            .any(|l| l.starts_with("Skipped watch:send-it-1:")));
        assert!(list_queued(dir.path()).is_empty());
        let left: Vec<String> = watches.saved().into_iter().map(|s| s.id).collect();
        assert_eq!(left, ["watch:send-it-1"]);
    }

    #[test]
    fn a_stop_waits_while_a_start_holds_the_intake() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = std::sync::Arc::new(Inbox::new(dir.path().join("inbox")));
        let watches = Watches::new(inbox, dir.path(), std::time::Duration::from_secs(60));
        let (done, finished) = std::sync::mpsc::channel();
        let held = watches.intake();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                let text = stop_text(&watches, dir.path(), &StopTarget::All, Utc::now());
                done.send(text).unwrap();
            });
            assert!(finished
                .recv_timeout(std::time::Duration::from_millis(150))
                .is_err());
            drop(held);
            assert_eq!(
                finished
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap(),
                "No active watches."
            );
        });
    }

    #[test]
    fn a_take_and_a_remove_never_both_win_the_same_request() {
        for round in 0..2000 {
            let dir = tempfile::tempdir().unwrap();
            queue(dir.path(), &request("watch:a")).unwrap();
            let gate = std::sync::Barrier::new(2);
            let (taken, removed) = std::thread::scope(|scope| {
                let take_side = scope.spawn(|| {
                    gate.wait();
                    take_all(dir.path()).len()
                });
                let remove_side = scope.spawn(|| {
                    gate.wait();
                    usize::from(remove_queued(dir.path(), "watch:a").is_ok())
                });
                (take_side.join().unwrap(), remove_side.join().unwrap())
            });
            assert_eq!(taken + removed, 1, "round {round}");
        }
    }

    #[test]
    fn a_missing_directory_has_no_requests() {
        let dir = tempfile::tempdir().unwrap();
        assert!(take_all(dir.path()).is_empty());
    }
}
