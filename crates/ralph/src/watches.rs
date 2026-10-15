use crate::subprocess::kill_process_group;
use crate::warn::{warn, warn_if_err};
use crate::worker_mcp::Inbox;
use ralph_core::watch::{
    build_report, StopReason, Tail, WatchOutcome, WatchSpec, TAIL_MAX_BYTES, TAIL_MAX_LINES,
};
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader};
use std::os::unix::process::{CommandExt, ExitStatusExt};
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
    #[error("a watch named `{0}` is already running")]
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
    stop: StopSlot,
    waiter: JoinHandle<()>,
}

#[derive(Clone)]
pub struct Watches {
    inbox: Arc<Inbox>,
    active: Arc<Mutex<HashMap<String, Active>>>,
}

struct Waiter {
    spec: WatchSpec,
    child: Child,
    tail: Arc<Mutex<Tail>>,
    drained: Receiver<()>,
    stop: StopSlot,
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
            if let Some(reason) = *locked(&self.stop) {
                kill_process_group(&mut self.child);
                return reason.outcome();
            }
            if let Some(limit) = self
                .spec
                .timeout
                .filter(|limit| started.elapsed() >= *limit)
            {
                kill_process_group(&mut self.child);
                return Some(WatchOutcome::TimedOut(limit));
            }
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    kill_process_group(&mut self.child);
                    return Some(WatchOutcome::from_exit(status.code(), status.signal()));
                }
                Ok(None) => thread::sleep(POLL),
                Err(e) => {
                    kill_process_group(&mut self.child);
                    return Some(WatchOutcome::WaitFailed(e.to_string()));
                }
            }
        }
    }

    fn run(mut self, inbox: &Inbox) {
        if let Some(outcome) = self.wait_for_outcome() {
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
    pub fn new(inbox: Arc<Inbox>) -> Self {
        Self {
            inbox,
            active: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[allow(dead_code, reason = "GUZ-485 adds the MCP watch tool that calls start")]
    pub fn start(&self, spec: WatchSpec) -> Result<(), WatchError> {
        let mut active = locked(&self.active);
        if active.contains_key(&spec.id) {
            return Err(WatchError::Duplicate(spec.id));
        }
        let (child, pipe) = spawn_child(&spec.command)?;
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
                stop,
                waiter: handle,
            },
        );
        Ok(())
    }

    #[allow(dead_code, reason = "GUZ-486 adds the /watch command that calls list")]
    pub fn list(&self) -> Vec<WatchSpec> {
        let mut specs: Vec<WatchSpec> = locked(&self.active)
            .values()
            .map(|entry| entry.spec.clone())
            .collect();
        specs.sort_by(|a, b| a.id.cmp(&b.id));
        specs
    }

    #[allow(dead_code, reason = "GUZ-486 adds the /watch command that calls stop")]
    pub fn stop(&self, id: &str) -> bool {
        let entry = locked(&self.active).remove(id);
        self.end(entry, StopReason::Requested)
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
