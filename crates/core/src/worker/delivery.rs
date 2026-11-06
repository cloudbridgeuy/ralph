use std::time::Duration;

pub const REPLY_CONFIRM: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Uptake {
    Busy,
    Reported,
    Taken,
    Silent,
    Lost,
}

impl Uptake {
    pub fn settled(self) -> bool {
        matches!(self, Self::Busy | Self::Reported | Self::Lost)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Probe {
    pub pane_busy: Option<bool>,
    pub entry_pending: Option<bool>,
    pub running: bool,
    pub sent: bool,
    pub lost: bool,
}

pub fn uptake(probe: &Probe) -> Uptake {
    if probe.lost {
        Uptake::Lost
    } else if !probe.sent {
        Uptake::Reported
    } else if probe.pane_busy == Some(true) || probe.running {
        Uptake::Busy
    } else if probe.entry_pending == Some(false) {
        Uptake::Taken
    } else {
        Uptake::Silent
    }
}

pub fn delivery_line(id: &str, uptake: Uptake, waited: Duration) -> String {
    let secs = (waited + Duration::from_millis(500)).as_secs();
    match uptake {
        Uptake::Busy => format!("Sent to worker {id}; busy."),
        Uptake::Reported => format!("Sent to worker {id}; reported."),
        Uptake::Taken => format!("Sent to worker {id}; taken, not started after {secs}s."),
        Uptake::Silent => format!("Sent to worker {id}; no reaction after {secs}s."),
        Uptake::Lost => format!("Not delivered to worker {id}: it is not running."),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueuedWorker {
    Absent,
    Alive,
    Exited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueuedOwner {
    Missing,
    Mine,
    Foreign,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueuedFacts {
    pub worker: QueuedWorker,
    pub starting: bool,
    pub owner: QueuedOwner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Queued {
    Deliver,
    Hold,
    Lost,
}

pub fn queued_disposition(facts: &QueuedFacts) -> Queued {
    match (facts.worker, facts.starting, facts.owner) {
        (QueuedWorker::Alive, _, _) => Queued::Deliver,
        (QueuedWorker::Exited, _, _) | (_, false, QueuedOwner::Missing) => Queued::Lost,
        _ => Queued::Hold,
    }
}

pub fn lost_marker_name(entry: &str) -> String {
    format!(".{entry}.lost")
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDLE: Probe = Probe {
        pane_busy: None,
        entry_pending: None,
        running: false,
        sent: true,
        lost: false,
    };

    fn with(edit: impl FnOnce(&mut Probe)) -> Uptake {
        let mut probe = IDLE;
        edit(&mut probe);
        uptake(&probe)
    }

    #[test]
    fn a_busy_pane_is_busy() {
        assert_eq!(with(|p| p.pane_busy = Some(true)), Uptake::Busy);
    }

    #[test]
    fn a_pane_that_is_not_busy_with_sent_still_there_is_silent() {
        assert_eq!(with(|p| p.pane_busy = Some(false)), Uptake::Silent);
    }

    #[test]
    fn a_running_turn_is_busy() {
        assert_eq!(with(|p| p.running = true), Uptake::Busy);
    }

    #[test]
    fn a_consumed_entry_without_a_turn_is_taken() {
        assert_eq!(with(|p| p.entry_pending = Some(false)), Uptake::Taken);
    }

    #[test]
    fn a_pending_entry_is_silent() {
        assert_eq!(with(|p| p.entry_pending = Some(true)), Uptake::Silent);
    }

    #[test]
    fn a_consumed_entry_with_a_running_turn_is_busy() {
        assert_eq!(
            with(|p| {
                p.entry_pending = Some(false);
                p.running = true;
            }),
            Uptake::Busy
        );
    }

    #[test]
    fn a_missing_sent_marker_is_reported_over_everything() {
        assert_eq!(
            with(|p| {
                p.sent = false;
                p.running = true;
                p.pane_busy = Some(true);
            }),
            Uptake::Reported
        );
    }

    #[test]
    fn only_busy_and_reported_stop_the_poll() {
        assert!(Uptake::Busy.settled() && Uptake::Reported.settled());
        assert!(!Uptake::Taken.settled() && !Uptake::Silent.settled());
    }

    #[test]
    fn each_uptake_has_its_line() {
        let waited = Duration::from_millis(5020);
        let line = |uptake| delivery_line("GUZ-1", uptake, waited);
        assert_eq!(line(Uptake::Busy), "Sent to worker GUZ-1; busy.");
        assert_eq!(line(Uptake::Reported), "Sent to worker GUZ-1; reported.");
        assert_eq!(
            line(Uptake::Taken),
            "Sent to worker GUZ-1; taken, not started after 5s."
        );
        assert_eq!(
            line(Uptake::Silent),
            "Sent to worker GUZ-1; no reaction after 5s."
        );
    }

    #[test]
    fn seconds_round_to_the_nearest() {
        let line = |ms| delivery_line("A", Uptake::Silent, Duration::from_millis(ms));
        assert_eq!(line(0), "Sent to worker A; no reaction after 0s.");
        assert_eq!(line(1400), "Sent to worker A; no reaction after 1s.");
        assert_eq!(line(1500), "Sent to worker A; no reaction after 2s.");
    }

    fn facts(worker: QueuedWorker, starting: bool, owner: QueuedOwner) -> Queued {
        queued_disposition(&QueuedFacts {
            worker,
            starting,
            owner,
        })
    }

    #[test]
    fn a_live_worker_gets_its_queued_entry() {
        let found = facts(QueuedWorker::Alive, false, QueuedOwner::Mine);
        assert_eq!(found, Queued::Deliver);
    }

    #[test]
    fn an_entry_waits_while_the_worker_starts() {
        let found = facts(QueuedWorker::Absent, true, QueuedOwner::Missing);
        assert_eq!(found, Queued::Hold);
    }

    #[test]
    fn an_entry_waits_while_this_chat_owns_an_unadopted_worker() {
        let found = facts(QueuedWorker::Absent, false, QueuedOwner::Mine);
        assert_eq!(found, Queued::Hold);
    }

    #[test]
    fn an_entry_another_chat_owns_is_left_alone() {
        let found = facts(QueuedWorker::Absent, false, QueuedOwner::Foreign);
        assert_eq!(found, Queued::Hold);
    }

    #[test]
    fn an_entry_for_an_exited_worker_is_lost() {
        let found = facts(QueuedWorker::Exited, false, QueuedOwner::Mine);
        assert_eq!(found, Queued::Lost);
    }

    #[test]
    fn an_entry_with_no_owner_is_lost() {
        let found = facts(QueuedWorker::Absent, false, QueuedOwner::Missing);
        assert_eq!(found, Queued::Lost);
    }

    #[test]
    fn a_lost_entry_beats_taken_and_settles() {
        let found = with(|p| {
            p.lost = true;
            p.entry_pending = Some(false);
        });
        assert_eq!(found, Uptake::Lost);
        assert!(Uptake::Lost.settled());
    }

    #[test]
    fn a_lost_delivery_line_says_not_delivered() {
        assert_eq!(
            delivery_line("GUZ-1", Uptake::Lost, Duration::ZERO),
            "Not delivered to worker GUZ-1: it is not running."
        );
    }
}
