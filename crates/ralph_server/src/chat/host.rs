use crate::chat::keepalive::KeepaliveWatch;
use crate::chat::monitor::{Monitor, POLL};
use crate::chat::schedule::fire_due;
use crate::chat::schedule_worker::tick_workers;
use crate::version_drift::DailyTick;
use crate::watch_requests;
use ralph_core::program_status;
use ralph_core::worker::{Report, INBOX_DIR};
use ralph_core::worker_check::check_every_polls;
use std::sync::Arc;

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

pub fn spawn_host(
    monitor: Arc<Monitor>,
    keepalive: Option<KeepaliveWatch>,
    sink: impl Fn(HostEvent) + Send + 'static,
) {
    std::thread::spawn(move || run_ticks(&monitor, &sink));
    if let Some(watch) = keepalive {
        std::thread::spawn(move || watch.run());
    }
}

fn run_ticks(monitor: &Monitor, sink: &impl Fn(HostEvent)) -> ! {
    let check_every = monitor
        .worker_check_secs
        .map(|secs| check_every_polls(secs, POLL));
    let mut tick: u64 = 0;
    let mut daily = DailyTick::start();
    let inbox_dir = monitor.store_dir.join(INBOX_DIR);
    loop {
        std::thread::sleep(POLL);
        tick += 1;
        let due = due(tick, check_every);
        daily.tick(&inbox_dir);
        watch_requests::start_queued(&monitor.watches, &monitor.inbox, &monitor.store_dir);
        fire_due(&monitor.schedule, &monitor.watches, &monitor.inbox);
        tick_workers(monitor);
        let changed = monitor.refresh(due.memory);
        for event in monitor.tick_events(changed) {
            sink(event);
        }
        if due.check {
            monitor.check();
        }
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

    fn status() -> program_status::Report {
        program_status::Report {
            state: State::Idle,
            msg: None,
        }
    }

    fn report(worker: &str) -> Report {
        Report {
            worker: worker.to_string(),
            at: String::new(),
            status: "done".to_string(),
            text: String::new(),
        }
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
