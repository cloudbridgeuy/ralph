use crate::chat::check::CheckState;
use crate::chat::host::{tick_events, HostEvent};
use crate::chat::keepalive::Stamp;
use crate::chat::memory::{self, Store};
use crate::chat::settle;
use crate::chat::stage;
use crate::cursor_owner::{now_secs, write_file};
use crate::schedule_store::ScheduleStore;
use crate::warn::{warn, warn_if_err};
use crate::watches::{locked, Watches};
use crate::worker_mcp::Inbox;
use crate::workers::{capture_pane, opencode_active, running_workers, Workers};
use ralph_core::chat::ResetLimits;
use ralph_core::chat_backend::Caps;
use ralph_core::chat_hold::HoldFilter;
use ralph_core::chat_memory::entries_to_shift;
use ralph_core::chat_process::{process_cell, process_usage, ProcessEnv, ProcessUsage, UsageTail};
use ralph_core::chat_status::{
    latest_states, schedule_states, watch_states, worker_states, RunningWorker, Status,
};
use ralph_core::chat_thread_auto::split_close_reports;
use ralph_core::inbox::Selection;
use ralph_core::inbox_alert::fresh_reports;
use ralph_core::program_status::{self, report_for, Phase};
use ralph_core::schedule_resume::LostTurn;
use ralph_core::schedule_worker::new_scheduled_reports;
use ralph_core::worker::WorkerId;
use ralph_core::worker::{
    activity_path, blocked_path, gone_acknowledged, gone_path, group_alerts, liveness,
    marker_state, ready_path, rearm, reduce, render_gone, sent_path, unreported_path,
    vanished_claude, waiting_path, ActivityMark, HarnessId, Report, ReportKind, Scope, READY_DIR,
};
use ralph_core::worker_check::{
    last_reports, plan, reported_since_nudge, CheckAction, WorkerObservation, NUDGE_TEXT,
};
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const POLL: Duration = Duration::from_millis(500);

pub fn ready_ids(store_dir: &Path) -> Vec<String> {
    std::fs::read_dir(store_dir.join(READY_DIR))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

pub fn fresh_warnings(seen: &mut HashSet<String>, warnings: Vec<String>) -> Vec<String> {
    warnings
        .into_iter()
        .filter(|warning| seen.insert(warning.clone()))
        .collect()
}

#[derive(Default)]
pub struct ProcessMemo {
    usage: Option<ProcessUsage>,
    shift: Option<usize>,
}

pub struct Monitor {
    pub inbox: Arc<Inbox>,
    pub watches: Watches,
    pub schedule: ScheduleStore,
    pub store: Store,
    pub store_dir: PathBuf,
    pub status: Mutex<Status>,
    pub seen: Mutex<HashMap<String, ReportKind>>,
    pub active: Mutex<Option<HashSet<String>>>,
    pub recent_history_tokens: usize,
    pub worker_check_secs: Option<u64>,
    pub worker_quiet_secs: u64,
    pub auto_continue: bool,
    pub hold_progress: bool,
    pub workers: Workers,
    pub check: Mutex<CheckState>,
    pub known: Mutex<Vec<Report>>,
    pub fresh: Mutex<(Vec<Report>, usize)>,
    pub starting: Arc<Mutex<Vec<String>>>,
    pub resuming: Mutex<Vec<LostTurn>>,
    pub scheduled_reports: Mutex<Vec<Report>>,
    pub warned: Mutex<HashSet<String>>,
    pub unrecorded: Mutex<HashSet<String>>,
    pub live: Arc<AtomicBool>,
    pub last_ping: Stamp,
    pub run: String,
    pub reset_limits: ResetLimits,
    pub caps: Caps,
    pub process: Mutex<ProcessMemo>,
    pub usage_tail: Mutex<UsageTail>,
    pub phase: Mutex<Phase>,
    pub hold: Mutex<Option<HoldFilter>>,
}

impl Monitor {
    pub fn status(&self) -> Status {
        locked(&self.status).clone()
    }

    pub fn set_phase(&self, phase: Phase) {
        *locked(&self.phase) = phase;
    }

    pub fn set_hold(&self, hold: Option<HoldFilter>) {
        *locked(&self.hold) = hold;
    }

    pub fn terminal_report(&self, unread: usize) -> program_status::Report {
        let mut status = self.status();
        status.unread = unread;
        status.hold = locked(&self.hold).clone();
        report_for(*locked(&self.phase), &status)
    }

    pub fn tick_events(&self, changed: bool) -> Vec<HostEvent> {
        let (fresh, unread) = std::mem::take(&mut *locked(&self.fresh));
        tick_events(
            changed,
            self.terminal_report(self.status().unread),
            fresh,
            unread,
        )
    }

    pub fn drain(&self, selection: Selection) -> io::Result<Vec<Report>> {
        let (_, reports) = split_close_reports(self.inbox.drain(selection)?);
        self.note_scheduled(&reports);
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
        let pane = (matches!(worker.kind, HarnessId::ClaudeTmux)
            && !(blocked || waiting || (ready && reported)))
            .then(|| capture_pane(&worker.session))
            .flatten();
        let activity = std::fs::read_to_string(activity_path(&self.store_dir, id))
            .ok()
            .and_then(|text| ActivityMark::parse(&text));
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
            activity: activity.map(|mark| mark.state),
            activity_age_secs: activity.map_or(0, |mark| now_secs().saturating_sub(mark.at)),
            auto_continue: self.auto_continue,
        }
    }

    fn heartbeats(&self, state: &mut CheckState, running: &[RunningWorker]) -> Vec<CheckAction> {
        state
            .claude_dead
            .retain(|id| gone_path(&self.store_dir, id).exists());
        state.heartbeats.retain(|id, _| {
            state.claude_dead.contains(id) || running.iter().any(|worker| &worker.id == id)
        });
        let now = now_secs();
        let mut alerts = Vec::new();
        let ids: Vec<String> = running
            .iter()
            .filter(|w| w.kind != HarnessId::ClaudeTmux)
            .map(|w| w.id.clone())
            .chain(state.claude_dead.iter().cloned())
            .collect();
        for id in &ids {
            let read = |path: PathBuf| std::fs::read_to_string(path).ok();
            let activity = read(activity_path(&self.store_dir, id));
            let gone = read(gone_path(&self.store_dir, id));
            let blocked = blocked_path(&self.store_dir, id).exists();
            let (phase, events) = marker_state(activity.as_deref(), gone.as_deref(), blocked);
            let mut current = rearm(
                state.heartbeats.remove(id).unwrap_or_default(),
                gone.is_some(),
            );
            current.phase = phase;
            for event in &events {
                current = reduce(&current, event).0;
            }
            let (next, alert) = liveness(&current, now, self.worker_quiet_secs);
            if gone_acknowledged(&next) {
                std::fs::remove_file(gone_path(&self.store_dir, id)).ok();
            }
            state.heartbeats.insert(id.clone(), next);
            alerts.extend(alert.map(|alert| (id.clone(), alert)));
        }
        group_alerts(alerts)
            .into_iter()
            .map(|(workers, text)| CheckAction::Inbox(workers, text))
            .collect()
    }

    pub fn check(&self) {
        let running = running_workers(&self.store_dir);
        let actions = {
            let mut state = locked(&self.check);
            let observations: Vec<WorkerObservation> = running
                .iter()
                .map(|worker| self.observe(&mut state, worker))
                .collect();
            state
                .nudges
                .retain(|id, _| running.iter().any(|worker| &worker.id == id));
            let alive: HashSet<String> = running
                .iter()
                .filter(|worker| matches!(worker.kind, HarnessId::ClaudeTmux))
                .map(|worker| worker.id.clone())
                .collect();
            for id in vanished_claude(&state.claude_alive, &alive, &ready_ids(&self.store_dir)) {
                let marked = write_file(
                    &gone_path(&self.store_dir, &id),
                    &render_gone(&Scope::Worker, "tmux session ended"),
                );
                warn_if_err(marked, format!("marking worker {id} gone"));
                state.claude_dead.insert(id);
            }
            state.claude_alive = alive;
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
                    let report = WorkerId::parse(&id).map(|worker| {
                        Report::check(&worker, &chrono::Utc::now().to_rfc3339(), text)
                    });
                    warn_if_err(
                        report
                            .map_err(std::io::Error::other)
                            .and_then(|report| self.inbox.post(&report, 0)),
                        format!("posting check for worker {id}"),
                    );
                }
            }
        }
        if self.auto_continue {
            self.stall_tick(&running);
        }
    }

    fn measure_shift(&self) -> Option<usize> {
        let (entries, _, warnings) = self.store.load_quiet().ok()?;
        for warning in fresh_warnings(&mut locked(&self.warned), warnings) {
            warn(warning);
        }
        Some(entries_to_shift(&entries, self.recent_history_tokens))
    }

    fn read_usage_rows(&self) -> Option<Option<ProcessUsage>> {
        let mut tail = locked(&self.usage_tail);
        let read = memory::read_usage_since(&memory::usage_path(), tail.clone(), &self.run);
        *tail = read.ok()?;
        Some(process_usage(&tail.rows))
    }

    fn reload_process(&self) {
        let usage = self.read_usage_rows();
        let shift = self.measure_shift();
        let mut memo = locked(&self.process);
        if let Some(usage) = usage {
            memo.usage = usage;
        }
        memo.shift = shift.or(memo.shift);
    }

    fn alert_fresh(&self, pending: &[Report]) {
        let mut known = locked(&self.known);
        let fresh: Vec<Report> = fresh_reports(&known, pending)
            .into_iter()
            .cloned()
            .collect();
        if !fresh.is_empty() {
            let mut queued = locked(&self.fresh);
            queued.0.extend(fresh);
            queued.1 = pending.len();
        }
        *known = pending.to_vec();
    }

    fn note_scheduled(&self, reports: &[Report]) {
        let mut known = locked(&self.scheduled_reports);
        let fresh = new_scheduled_reports(&known, reports);
        known.extend(fresh);
    }

    pub fn refresh(&self, with_memory: bool) -> bool {
        let peeked = self.inbox.peek();
        if let Ok(pending) = &peeked {
            self.alert_fresh(pending);
            self.note_scheduled(pending);
        }
        let (_, pending) = split_close_reports(peeked.unwrap_or_default());
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
        if with_memory {
            self.reload_process();
        }
        let process = {
            let memo = locked(&self.process);
            process_cell(
                memo.usage.as_ref(),
                &ProcessEnv {
                    now: chrono::Utc::now(),
                    last_ping: *locked(&self.last_ping),
                    live: self.live.load(Ordering::SeqCst),
                    limits: self.reset_limits,
                    shift: memo.shift,
                    caps: self.caps,
                },
            )
        };
        let mut status = locked(&self.status);
        let next = Status {
            unread: pending.len(),
            closes: settle::pending(&self.store_dir),
            workers,
            watches: watch_states(&self.watches.list(), chrono::Utc::now()),
            schedule: schedule_states(
                &self.schedule.load_all(),
                chrono::Utc::now(),
                *chrono::Local::now().offset(),
            ),
            process: Some(process),
            napping: self.store.napping(),
            queue: None,
            hold: None,
            stage: stage::load(&self.store_dir).map_or(0, |stage| stage.items.len()),
        };
        let changed = *status != next;
        *status = next;
        changed
    }
}
