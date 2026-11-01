use std::time::Duration;

pub const REPLY_CONFIRM: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Uptake {
    Busy,
    Reported,
    Taken,
    Silent,
}

impl Uptake {
    pub fn settled(self) -> bool {
        matches!(self, Self::Busy | Self::Reported)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Probe {
    pub pane_busy: Option<bool>,
    pub entry_pending: Option<bool>,
    pub running: bool,
    pub sent: bool,
}

pub fn uptake(probe: &Probe) -> Uptake {
    if !probe.sent {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDLE: Probe = Probe {
        pane_busy: None,
        entry_pending: None,
        running: false,
        sent: true,
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
}
