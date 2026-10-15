use crate::warn::{warn, warn_if_err};
use crate::watches::Watches;
use crate::worker_mcp::Inbox;
use ralph_core::watch_request::{
    already_queued_message, plan_queued, start_failure_report, WatchRequest, WATCH_REQUESTS_DIR,
};
use ralph_core::worker::Report;
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

fn is_request(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "json")
        && path
            .file_name()
            .is_some_and(|name| !name.to_string_lossy().starts_with('.'))
}

fn take(path: &Path) -> Option<WatchRequest> {
    let parsed = fs::read(path)
        .map_err(|e| e.to_string())
        .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|e| e.to_string()));
    warn_if_err(
        fs::remove_file(path),
        format!("removing {}", path.display()),
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
    fn a_missing_directory_has_no_requests() {
        let dir = tempfile::tempdir().unwrap();
        assert!(take_all(dir.path()).is_empty());
    }
}
