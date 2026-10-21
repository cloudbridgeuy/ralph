use crate::chat::check::CheckState;
use crate::chat::memory::Store;
use crate::cursor_owner::now_secs;
use crate::inbox_alert;
use crate::signal;
use crate::subprocess::Session;
use crate::warn::{warn, warn_if_err};
use crate::watch_requests;
use crate::watches::{locked, Watches};
use crate::worker_mcp::Inbox;
use crate::workers::headless::{ExitCleanup, HeadlessShutdown};
use crate::workers::{capture_pane, opencode_active, running_workers, Workers};
use ralph_core::chat::VIEW_LINES;
use ralph_core::chat_input::{input_lost_at_prompt, InputWatch};
use ralph_core::chat_memory::split_view;
use ralph_core::chat_status::{
    last_turn_cache, latest_states, memory_size, watch_states, worker_states, Memory,
    RunningWorker, Status,
};
use ralph_core::inbox::Selection;
use ralph_core::inbox_alert::{fresh_reports, inbox_alert, AlertConfig};
use ralph_core::worker::{
    activity_path, blocked_path, gone_acknowledged, gone_path, group_alerts, liveness,
    marker_state, ready_path, rearm, reduce, sent_path, unreported_path, waiting_path, HarnessId,
    Report, HANDOFF_POLL, READY_DIR,
};
use ralph_core::worker_check::{
    check_every_polls, last_reports, plan, reported_since_nudge, CheckAction, WorkerObservation,
    CHECK_STATUS, NUDGE_TEXT,
};
use std::collections::{HashMap, HashSet};
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const POLL: Duration = Duration::from_millis(500);
const MEMORY_EVERY_POLLS: u64 = 10;
const INPUT_LOST_EXIT: i32 = 129;

pub(super) fn ready_ids(store_dir: &Path) -> Vec<String> {
    std::fs::read_dir(store_dir.join(READY_DIR))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn fresh_warnings(seen: &mut HashSet<String>, warnings: Vec<String>) -> Vec<String> {
    warnings
        .into_iter()
        .filter(|warning| seen.insert(warning.clone()))
        .collect()
}

pub(super) struct Monitor {
    pub(super) inbox: Arc<Inbox>,
    pub(super) watches: Watches,
    pub(super) store: Store,
    pub(super) store_dir: PathBuf,
    pub(super) status: Mutex<Status>,
    pub(super) seen: Mutex<HashMap<String, String>>,
    pub(super) active: Mutex<Option<HashSet<String>>>,
    pub(super) recent_history_tokens: usize,
    pub(super) worker_check_secs: Option<u64>,
    pub(super) worker_quiet_secs: u64,
    pub(super) workers: Workers,
    pub(super) check: Mutex<CheckState>,
    pub(super) known: Mutex<Vec<Report>>,
    pub(super) warned: Mutex<HashSet<String>>,
    pub(super) alert: AlertConfig,
}

impl Monitor {
    pub(super) fn status(&self) -> Status {
        locked(&self.status).clone()
    }

    pub(super) fn drain(&self, selection: Selection) -> io::Result<Vec<Report>> {
        let reports = self.inbox.drain(selection)?;
        locked(&self.seen).extend(latest_states(&reports));
        locked(&self.check).reports.extend(last_reports(&reports));
        Ok(reports)
    }

    fn observe(&self, state: &mut CheckState, worker: &RunningWorker) -> WorkerObservation {
        let id = &worker.id;
        let ready = ready_path(&self.store_dir, id).exists();
        let blocked = blocked_path(&self.store_dir, id).exists();
        let waiting = waiting_path(&self.store_dir, id).exists();
        let unreported = unreported_path(&self.store_dir, id).exists();
        let reported = !sent_path(&self.store_dir, id).exists()
            && reported_since_nudge(state.reports.get(id), state.nudges.get(id));
        let pane = (matches!(worker.kind, HarnessId::Claude)
            && !(blocked || waiting || (ready && reported)))
            .then(|| capture_pane(&worker.session))
            .flatten();
        let pane_changed = match &pane {
            Some(pane) => state.panes.insert(id.clone(), pane.clone()).as_ref() != Some(pane),
            None => {
                state.panes.remove(id);
                true
            }
        };
        WorkerObservation {
            id: id.clone(),
            session: worker.session.clone(),
            kind: worker.kind,
            ready,
            blocked,
            waiting,
            unreported,
            reported_since_nudge: reported,
            pane,
            pane_changed,
        }
    }

    fn heartbeats(&self, state: &mut CheckState, running: &[RunningWorker]) -> Vec<CheckAction> {
        state
            .heartbeats
            .retain(|id, _| running.iter().any(|worker| &worker.id == id));
        let now = now_secs();
        let mut alerts = Vec::new();
        for worker in running
            .iter()
            .filter(|w| matches!(w.kind, HarnessId::Cursor | HarnessId::Opencode))
        {
            let read = |path: PathBuf| std::fs::read_to_string(path).ok();
            let activity = read(activity_path(&self.store_dir, &worker.id));
            let gone = read(gone_path(&self.store_dir, &worker.id));
            let blocked = blocked_path(&self.store_dir, &worker.id).exists();
            let (phase, events) = marker_state(activity.as_deref(), gone.as_deref(), blocked);
            let mut current = rearm(
                state.heartbeats.remove(&worker.id).unwrap_or_default(),
                gone.is_some(),
            );
            current.phase = phase;
            for event in &events {
                current = reduce(&current, event).0;
            }
            let (next, alert) = liveness(&current, now, self.worker_quiet_secs);
            if gone_acknowledged(&next) {
                std::fs::remove_file(gone_path(&self.store_dir, &worker.id)).ok();
            }
            state.heartbeats.insert(worker.id.clone(), next);
            alerts.extend(alert.map(|alert| (worker.id.clone(), alert)));
        }
        group_alerts(alerts)
            .into_iter()
            .map(|(workers, text)| CheckAction::Inbox(workers, text))
            .collect()
    }

    fn check(&self) {
        let running = running_workers(&self.store_dir);
        let actions = {
            let mut state = locked(&self.check);
            let observations: Vec<WorkerObservation> = running
                .iter()
                .map(|worker| self.observe(&mut state, worker))
                .collect();
            state
                .panes
                .retain(|id, _| running.iter().any(|worker| &worker.id == id));
            state
                .nudges
                .retain(|id, _| running.iter().any(|worker| &worker.id == id));
            let (mut actions, memory) = plan(&state.memory, &observations);
            state.memory = memory;
            actions.extend(self.heartbeats(&mut state, &running));
            for action in &actions {
                if let CheckAction::Nudge(id) = action {
                    let last = state.reports.get(id).cloned();
                    state.nudges.insert(id.clone(), last);
                }
            }
            actions
        };
        for action in actions {
            match action {
                CheckAction::Nudge(id) => warn_if_err(
                    self.workers.nudge(&id, NUDGE_TEXT),
                    format!("nudging worker {id}"),
                ),
                CheckAction::Inbox(id, text) => {
                    let report = Report {
                        worker: id.clone(),
                        at: chrono::Utc::now().to_rfc3339(),
                        status: CHECK_STATUS.to_string(),
                        text,
                    };
                    warn_if_err(
                        self.inbox.post(&report, 0),
                        format!("posting check for worker {id}"),
                    );
                }
            }
        }
    }

    fn measure_memory(&self) -> Option<Memory> {
        let (entries, tree, warnings) = self.store.load_quiet().ok()?;
        for warning in fresh_warnings(&mut locked(&self.warned), warnings) {
            warn(warning);
        }
        let split = split_view(&entries, &tree, VIEW_LINES, self.recent_history_tokens);
        Some(memory_size(
            entries.len(),
            &[split.older, split.tail].concat(),
        ))
    }

    fn alert_fresh(&self, pending: &[Report]) {
        let mut known = locked(&self.known);
        let fresh = fresh_reports(&known, pending);
        if !fresh.is_empty() && self.alert.enabled() {
            if let Some(alert) = inbox_alert(&fresh, inbox_alert::focus(), &self.alert) {
                inbox_alert::deliver(&alert);
            }
        }
        *known = pending.to_vec();
    }

    pub(super) fn refresh(&self, with_memory: bool) -> bool {
        let peeked = self.inbox.peek();
        if let Ok(pending) = &peeked {
            self.alert_fresh(pending);
        }
        let pending = peeked.unwrap_or_default();
        locked(&self.check).reports.extend(last_reports(&pending));
        let seen = {
            let mut seen = locked(&self.seen);
            seen.extend(latest_states(&pending));
            seen.clone()
        };
        let running = running_workers(&self.store_dir);
        let active = {
            let mut active = locked(&self.active);
            if with_memory {
                *active = running
                    .iter()
                    .any(|worker| worker.kind == HarnessId::Opencode)
                    .then(opencode_active)
                    .flatten();
            }
            active.clone()
        };
        let sent: HashSet<String> = running
            .iter()
            .filter(|worker| sent_path(&self.store_dir, &worker.id).exists())
            .map(|worker| worker.id.clone())
            .collect();
        let blocked: HashSet<String> = running
            .iter()
            .filter(|worker| blocked_path(&self.store_dir, &worker.id).exists())
            .map(|worker| worker.id.clone())
            .collect();
        let workers = worker_states(&running, &seen, &sent, active.as_ref(), &blocked);
        let memory = if with_memory {
            self.measure_memory()
        } else {
            None
        };
        let turn = if with_memory {
            self.store
                .load_usage()
                .ok()
                .map(|text| last_turn_cache(&text))
        } else {
            None
        };
        let mut status = locked(&self.status);
        let next = Status {
            unread: pending.len(),
            workers,
            watches: watch_states(&self.watches.list(), chrono::Utc::now()),
            memory: memory.unwrap_or(status.memory),
            turn: turn.unwrap_or(status.turn),
            napping: self.store.napping(),
        };
        let changed = *status != next;
        *status = next;
        changed
    }
}

pub(super) fn watch_status(monitor: Arc<Monitor>, repaint: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        let check_every = monitor
            .worker_check_secs
            .map(|secs| check_every_polls(secs, POLL));
        let mut tick: u64 = 0;
        loop {
            std::thread::sleep(POLL);
            tick += 1;
            watch_requests::start_queued(&monitor.watches, &monitor.inbox, &monitor.store_dir);
            if monitor.refresh(tick.is_multiple_of(MEMORY_EVERY_POLLS)) {
                repaint();
            }
            if check_every.is_some_and(|every| tick.is_multiple_of(every)) {
                monitor.check();
            }
        }
    });
}

pub(super) fn watch_headless(store_dir: PathBuf) -> HeadlessShutdown {
    let headless = HeadlessShutdown::new(store_dir);
    for host in headless.hosts() {
        let resumed = Arc::clone(host);
        std::thread::spawn(move || resumed.resume());
        let polled = Arc::clone(host);
        // Keep each host independent: HTTP and ACP handoffs can block, and neither
        // should delay the other host or the status/check loop.
        std::thread::spawn(move || loop {
            std::thread::sleep(HANDOFF_POLL);
            if !polled.poll() {
                break;
            }
        });
    }
    headless
}

pub(super) fn stdin_hung_up() -> bool {
    let mut stdin = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: ralph's direct dependencies have no safe poll wrapper, and this is
    // the backup check for terminals that close without sending SIGHUP.
    // `stdin` is a valid, initialized pollfd on the stack and nfds is 1, so poll
    // only touches that one struct. fd 0 is not owned or closed here. A zero
    // timeout means poll never blocks. revents is read only when poll returns > 0.
    unsafe { libc::poll(&mut stdin, 1, 0) > 0 && stdin.revents & libc::POLLHUP != 0 }
}

pub(super) fn watch_input(
    at_prompt: Arc<AtomicBool>,
    session: Arc<Mutex<Option<Session>>>,
    plugin_dir: Option<PathBuf>,
    exit: ExitCleanup,
) {
    let started_on_terminal = io::stdin().is_terminal();
    std::thread::spawn(move || loop {
        std::thread::sleep(POLL);
        if input_lost_at_prompt(&InputWatch {
            hangup_signal: signal::is_terminated(),
            started_on_terminal,
            stdin_hung_up: stdin_hung_up(),
            at_prompt: at_prompt.load(Ordering::SeqCst),
        }) {
            // No turn runs at the prompt, so the lock is free; `exit` skips
            // destructors, so close the claude process group here.
            if let Ok(mut live) = session.try_lock() {
                live.take();
            }
            if let Some(dir) = &plugin_dir {
                std::fs::remove_dir_all(dir).ok();
            }
            exit.run();
            std::process::exit(INPUT_LOST_EXIT);
        }
    });
}
