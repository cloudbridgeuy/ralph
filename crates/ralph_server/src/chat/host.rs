use ralph_core::program_status;
use ralph_core::worker::Report;

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
}
