use crate::subprocess::kill_process_group;
use crate::warn::{warn, warn_if_err};
use crate::watch_store::WatchStore;
use crate::worker_mcp::Inbox;
use chrono::{DateTime, Utc};
use ralph_core::watch::{
    build_report, poll_decision, ChildState, PollDecision, StopReason, Tail, WatchKind,
    WatchOutcome, WatchSpec, TAIL_MAX_BYTES, TAIL_MAX_LINES,
};
use ralph_core::watch_command::{decide_stop, StopAction, StopResult};
use ralph_core::watch_request::watch_in_use_message;
use ralph_core::watch_store::{plan_restore, RestoreAction, SavedWatch, WATCHES_DIR};
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(25);
const DRAIN_GRACE: Duration = Duration::from_millis(500);

pub(crate) fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

type StopSlot = Arc<Mutex<Option<StopReason>>>;

#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    #[error("{}", watch_in_use_message(.0))]
    Duplicate(String),
    #[error("failed to start watch: {0}")]
    Spawn(#[from] io::Error),
}

pub struct CloseGuard(Watches);

impl Drop for CloseGuard {
    fn drop(&mut self) {
        self.0.kill_all();
    }
}

struct Active {
    spec: WatchSpec,
    started_at: DateTime<Utc>,
    stop: StopSlot,
    waiter: JoinHandle<()>,
}

#[derive(Clone)]
pub struct Watches {
    inbox: Arc<Inbox>,
    store: WatchStore,
    active: Arc<Mutex<HashMap<String, Active>>>,
    default_timeout: Duration,
}

struct Waiter {
    spec: WatchSpec,
    child: Child,
    tail: Arc<Mutex<Tail>>,
    drained: Receiver<()>,
    stop: StopSlot,
    store: WatchStore,
}

fn request_stop(slot: &StopSlot, reason: StopReason) {
    locked(slot).get_or_insert(reason);
}

fn capture(pipe: impl io::Read, tail: &Mutex<Tail>, drained: &Sender<()>) {
    let mut reader = BufReader::new(pipe);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let line = String::from_utf8_lossy(&buf);
                locked(tail).push_line(line.trim_end_matches(['\n', '\r']));
            }
        }
    }
    let _ = drained.send(());
}

impl Waiter {
    fn wait_for_outcome(&mut self) -> Option<WatchOutcome> {
        let started = Instant::now();
        loop {
            let stop = *locked(&self.stop);
            let child = match self.child.try_wait() {
                Ok(Some(status)) => ChildState::Exited {
                    code: status.code(),
                    signal: status.signal(),
                },
                Ok(None) => ChildState::Running,
                Err(e) => ChildState::WaitFailed(e.to_string()),
            };
            match poll_decision(&self.spec, started.elapsed(), stop, child) {
                PollDecision::Wait => thread::sleep(POLL),
                PollDecision::Finish(outcome) => {
                    kill_process_group(&mut self.child);
                    return outcome;
                }
            }
        }
    }

    fn run(mut self, inbox: &Inbox) {
        if let Some(outcome) = self.wait_for_outcome() {
            let persistent = self.spec.kind == WatchKind::Persistent;
            let _ = self.drained.recv_timeout(DRAIN_GRACE);
            let report = build_report(
                &self.spec,
                &outcome,
                &locked(&self.tail),
                &chrono::Utc::now().to_rfc3339(),
            );
            warn_if_err(
                inbox.post(&report, 0),
                format!("posting watch {}", self.spec.id),
            );
            if persistent {
                self.store.remove(&self.spec.id);
            }
        }
    }
}

fn spawn_child(command: &str) -> io::Result<(Child, io::PipeReader)> {
    let (reader, writer) = io::pipe()?;
    let child = Command::new("sh")
        .args(["-c", command])
        .stdin(Stdio::null())
        .stdout(writer.try_clone()?)
        .stderr(writer)
        .process_group(0)
        .spawn()?;
    Ok((child, reader))
}

impl Watches {
    pub fn new(inbox: Arc<Inbox>, store_dir: &Path, default_timeout: Duration) -> Self {
        Self {
            inbox,
            store: WatchStore::new(store_dir.join(WATCHES_DIR)),
            active: Arc::new(Mutex::new(HashMap::new())),
            default_timeout,
        }
    }

    pub fn start(&self, spec: WatchSpec) -> Result<(), WatchError> {
        let spec = spec.with_default_timeout(self.default_timeout);
        let saved = (spec.kind == WatchKind::Persistent)
            .then(|| SavedWatch::new(&spec, chrono::Utc::now()));
        self.launch(spec, saved.as_ref())
    }

    pub fn restore(&self) {
        let now = chrono::Utc::now();
        for action in plan_restore(&self.store.load_all(), now, self.default_timeout) {
            match action {
                RestoreAction::Restart(spec) => {
                    let id = spec.id.clone();
                    warn_if_err(self.launch(spec, None), format!("restoring watch {id}"));
                }
                RestoreAction::Report { spec, outcome } => {
                    self.report_now(&spec, &outcome, now);
                }
                RestoreAction::Discard(id) => self.store.remove(&id),
            }
        }
    }

    fn report_now(
        &self,
        spec: &WatchSpec,
        outcome: &WatchOutcome,
        now: chrono::DateTime<chrono::Utc>,
    ) {
        let tail = Tail::new(TAIL_MAX_LINES, TAIL_MAX_BYTES);
        let report = build_report(spec, outcome, &tail, &now.to_rfc3339());
        warn_if_err(
            self.inbox.post(&report, 0),
            format!("posting watch {}", spec.id),
        );
        self.store.remove(&spec.id);
    }

    fn launch(&self, spec: WatchSpec, fresh: Option<&SavedWatch>) -> Result<(), WatchError> {
        let mut active = locked(&self.active);
        if active.contains_key(&spec.id) {
            return Err(WatchError::Duplicate(spec.id));
        }
        if let Some(saved) = fresh {
            self.store.save(saved)?;
        }
        let (child, pipe) = spawn_child(&spec.command).inspect_err(|_| {
            if fresh.is_some() {
                self.store.remove(&spec.id);
            }
        })?;
        let tail = Arc::new(Mutex::new(Tail::new(TAIL_MAX_LINES, TAIL_MAX_BYTES)));
        let (drained_tx, drained) = mpsc::channel();
        let reader_tail = Arc::clone(&tail);
        thread::spawn(move || capture(pipe, &reader_tail, &drained_tx));
        let stop: StopSlot = Arc::new(Mutex::new(None));
        let waiter = Waiter {
            spec: spec.clone(),
            child,
            tail,
            drained,
            stop: Arc::clone(&stop),
            store: self.store.clone(),
        };
        let registry = Arc::clone(&self.active);
        let inbox = Arc::clone(&self.inbox);
        let id = spec.id.clone();
        let own_stop = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            waiter.run(&inbox);
            let mut active = locked(&registry);
            if active
                .get(&id)
                .is_some_and(|entry| Arc::ptr_eq(&entry.stop, &own_stop))
            {
                active.remove(&id);
            }
        });
        active.insert(
            spec.id.clone(),
            Active {
                spec,
                started_at: chrono::Utc::now(),
                stop,
                waiter: handle,
            },
        );
        Ok(())
    }

    pub fn list(&self) -> Vec<SavedWatch> {
        let mut watches: Vec<SavedWatch> = locked(&self.active)
            .values()
            .map(|entry| SavedWatch::new(&entry.spec, entry.started_at))
            .collect();
        watches.sort_by(|a, b| a.id.cmp(&b.id));
        watches
    }

    pub fn used_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = locked(&self.active).keys().cloned().collect();
        ids.extend(self.store.load_all().into_iter().map(|saved| saved.id));
        ids
    }

    pub fn stop(&self, id: &str) -> StopResult {
        let running: Vec<String> = locked(&self.active).keys().cloned().collect();
        let saved: Vec<String> = self
            .store
            .load_all()
            .into_iter()
            .map(|saved| saved.id)
            .collect();
        let action = decide_stop(id, &running, &saved);
        match action {
            StopAction::KillRunning => {
                let entry = locked(&self.active).remove(id);
                self.end(entry, StopReason::Requested);
            }
            StopAction::RemoveSaved => self.store.remove(id),
            StopAction::Unknown => {}
        }
        action.result()
    }

    pub fn close_guard(&self) -> CloseGuard {
        CloseGuard(self.clone())
    }

    pub fn kill_all(&self) {
        let entries: Vec<Active> = locked(&self.active).drain().map(|(_, e)| e).collect();
        for entry in entries {
            self.end(Some(entry), StopReason::Shutdown);
        }
    }

    fn end(&self, entry: Option<Active>, reason: StopReason) -> bool {
        let Some(entry) = entry else {
            return false;
        };
        request_stop(&entry.stop, reason);
        if entry.waiter.join().is_err() {
            warn(format!("watch {} thread panicked", entry.spec.id));
        }
        true
    }
}
