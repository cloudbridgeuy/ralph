use crate::chat::keepalive::KeepaliveWatch;
use crate::chat::monitor::{Monitor, POLL};
use crate::chat::schedule::fire_due;
use crate::chat::schedule_worker::tick_workers;
use crate::chat::session::Session;
use crate::version_drift::DailyTick;
use crate::watch_requests;
use crate::workers::usage_store;
use ralph_core::program_status;
use ralph_core::worker::{Report, INBOX_DIR};
use ralph_core::worker_check::check_every_polls;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

const MEMORY_EVERY_POLLS: u64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Due {
    pub memory: bool,
    pub check: bool,
}

pub fn due(tick: u64, check_every: Option<u64>) -> Due {
    Due {
        memory: tick.is_multiple_of(MEMORY_EVERY_POLLS),
        check: check_every.is_some_and(|every| tick.is_multiple_of(every)),
    }
}

pub struct HostGuard {
    stops: Vec<Sender<()>>,
    threads: Vec<JoinHandle<()>>,
}

impl Drop for HostGuard {
    fn drop(&mut self) {
        self.stops.clear();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

pub fn spawn_session_host(
    session: &Session,
    keepalive_hours: Option<u64>,
    idle: Arc<AtomicBool>,
    daily_drift: bool,
    sink: impl Fn(HostEvent) + Send + 'static,
) -> HostGuard {
    let turn = &session.turn;
    let keepalive = keepalive_hours.map(|limit_hours| KeepaliveWatch {
        limit_hours,
        idle,
        session: Arc::clone(&turn.session),
        live: Arc::clone(&turn.live),
        last_turn_end: Arc::clone(&turn.last_turn_end),
        last_ping: Arc::clone(&session.core.last_ping),
        last_attempt: Arc::default(),
        keeps: Mutex::default(),
        identity: Arc::clone(&turn.identity),
        run: turn.run.clone(),
        store: usage_store(&turn.store_dir),
        caps: turn.backend.capabilities(),
    });
    let mut guard = HostGuard {
        stops: Vec::new(),
        threads: Vec::new(),
    };
    let (stop, receiver) = mpsc::channel();
    guard.stops.push(stop);
    let monitor = Arc::clone(&session.core);
    guard.threads.push(std::thread::spawn(move || {
        run_ticks(&monitor, &sink, &receiver, daily_drift);
    }));
    if let Some(watch) = keepalive {
        let (stop, receiver) = mpsc::channel();
        guard.stops.push(stop);
        guard
            .threads
            .push(std::thread::spawn(move || watch.run(&receiver)));
    }
    guard
}

pub(crate) fn wait_for_tick(stop: &Receiver<()>, interval: Duration) -> bool {
    matches!(stop.recv_timeout(interval), Err(RecvTimeoutError::Timeout))
}

fn run_ticks(monitor: &Monitor, sink: &impl Fn(HostEvent), stop: &Receiver<()>, daily_drift: bool) {
    let check_every = monitor
        .worker_check_secs
        .map(|secs| check_every_polls(secs, POLL));
    let mut tick: u64 = 0;
    let mut daily = daily_drift.then(DailyTick::start);
    let mut starts = Vec::new();
    let inbox_dir = monitor.store_dir.join(INBOX_DIR);
    while wait_for_tick(stop, POLL) {
        tick += 1;
        let due = due(tick, check_every);
        if let Some(daily) = &mut daily {
            daily.tick(&inbox_dir);
        }
        watch_requests::start_queued(&monitor.watches, &monitor.inbox, &monitor.store_dir);
        fire_due(&monitor.schedule, &monitor.watches, &monitor.inbox);
        tick_workers(monitor, &mut starts);
        let changed = monitor.refresh(due.memory);
        for event in monitor.tick_events(changed) {
            sink(event);
        }
        if due.check {
            monitor.check();
        }
    }
    for start in starts {
        let _ = start.join();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostEvent {
    Fresh { fresh: Vec<Report>, unread: usize },
    Repaint,
    Status(program_status::Report),
}

pub fn tick_events(
    changed: bool,
    status: program_status::Report,
    fresh: Vec<Report>,
    unread: usize,
) -> Vec<HostEvent> {
    let mut events = Vec::new();
    if !fresh.is_empty() {
        events.push(HostEvent::Fresh { fresh, unread });
    }
    if changed {
        events.push(HostEvent::Repaint);
    }
    events.push(HostEvent::Status(status));
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::program_status::State;

    #[test]
    fn host_owns_maintenance() {
        fn owns_output<F, T>(_: F) -> bool
        where
            F: FnOnce(&Session, Option<u64>, Arc<AtomicBool>, bool, fn(HostEvent)) -> T,
        {
            std::mem::needs_drop::<T>()
        }
        assert!(owns_output(spawn_session_host));
    }

    #[test]
    fn stopping_one_loop_does_not_stop_its_sibling() {
        let (first, first_stop) = mpsc::channel();
        let (second, second_stop) = mpsc::channel();
        drop(first);
        assert!(!wait_for_tick(&first_stop, Duration::ZERO));
        assert!(wait_for_tick(&second_stop, Duration::ZERO));
        second.send(()).unwrap();
        assert!(!wait_for_tick(&second_stop, Duration::ZERO));
    }

    fn status() -> program_status::Report {
        program_status::Report {
            state: State::Idle,
            msg: None,
        }
    }

    fn report(worker: &str) -> Report {
        Report::from_worker(
            worker,
            "",
            ralph_core::worker::ReportStatus::Done,
            String::new(),
        )
    }

    #[test]
    fn tick_events_quiet_tick_emits_only_status() {
        assert_eq!(
            tick_events(false, status(), Vec::new(), 0),
            vec![HostEvent::Status(status())]
        );
    }

    #[test]
    fn tick_events_changed_tick_repaints_before_status() {
        assert_eq!(
            tick_events(true, status(), Vec::new(), 0),
            vec![HostEvent::Repaint, HostEvent::Status(status())]
        );
    }

    #[test]
    fn tick_events_fresh_reports_come_first() {
        let fresh = vec![report("w1")];
        assert_eq!(
            tick_events(true, status(), fresh.clone(), 2),
            vec![
                HostEvent::Fresh { fresh, unread: 2 },
                HostEvent::Repaint,
                HostEvent::Status(status())
            ]
        );
    }

    #[test]
    fn due_reloads_memory_every_tenth_tick() {
        assert!(due(10, None).memory);
        assert!(due(20, None).memory);
        assert!(!due(9, None).memory);
    }

    #[test]
    fn due_checks_only_on_multiples_of_the_check_interval() {
        assert!(due(4, Some(2)).check);
        assert!(!due(3, Some(2)).check);
        assert!(!due(4, None).check);
    }
}
