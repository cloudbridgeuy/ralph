use ralph_core::worker::InterruptOutcome;

const LIMIT: usize = 128;

#[derive(Default)]
pub(super) struct Cancellation {
    entries: Vec<String>,
}

impl Cancellation {
    pub(super) fn request(
        &mut self,
        entry: String,
        target: u64,
        current: u64,
        send: impl FnOnce() -> bool,
    ) -> Option<InterruptOutcome> {
        if target.is_multiple_of(2) {
            return Some(InterruptOutcome::Sent);
        }
        if target != current {
            return Some(InterruptOutcome::Completed);
        }
        if self.entries.len() == LIMIT {
            return Some(InterruptOutcome::Dropped);
        }
        if self.entries.is_empty() && !send() {
            return Some(InterruptOutcome::Dropped);
        }
        self.entries.push(entry);
        None
    }

    pub(super) fn pending(&self) -> bool {
        !self.entries.is_empty()
    }

    pub(super) fn finish(
        &mut self,
        reason: Result<&str, &str>,
        closing: bool,
    ) -> Vec<(String, InterruptOutcome)> {
        let outcome = match (closing, reason) {
            (true, _) => InterruptOutcome::Gone,
            (false, Ok("cancelled")) => InterruptOutcome::Cancelled,
            (false, Ok(_)) => InterruptOutcome::Completed,
            (false, Err(_)) => InterruptOutcome::Unknown,
        };
        self.entries
            .drain(..)
            .map(|entry| (entry, outcome))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn cancellation_waits_for_terminal_result_and_deduplicates() {
        let mut pending = Cancellation::default();
        let sends = Cell::new(0);
        let send = || {
            sends.set(sends.get() + 1);
            true
        };
        assert_eq!(pending.request("a".into(), 1, 1, send), None);
        assert_eq!(pending.request("b".into(), 1, 1, send), None);
        assert_eq!(sends.get(), 1);
        assert_eq!(
            pending.finish(Ok("cancelled"), false),
            vec![
                ("a".into(), InterruptOutcome::Cancelled),
                ("b".into(), InterruptOutcome::Cancelled)
            ]
        );
        assert!(pending.finish(Ok("cancelled"), false).is_empty());
    }

    #[test]
    fn idle_late_and_send_failure_do_not_cancel_a_new_turn() {
        let mut pending = Cancellation::default();
        let effects = Cell::new(0);
        let side_effect = || {
            effects.set(effects.get() + 1);
            true
        };
        assert_eq!(
            pending.request("idle".into(), 0, 1, side_effect),
            Some(InterruptOutcome::Sent)
        );
        assert_eq!(
            pending.request("late".into(), 1, 3, side_effect),
            Some(InterruptOutcome::Completed)
        );
        assert_eq!(effects.get(), 0);
        assert_eq!(
            pending.request("failed".into(), 3, 3, || false),
            Some(InterruptOutcome::Dropped)
        );
        assert!(!pending.pending());
        assert_eq!(pending.request("sent".into(), 3, 3, || true), None);
        assert_eq!(
            pending.finish(Ok("end_turn"), false),
            vec![("sent".into(), InterruptOutcome::Completed)]
        );
        assert_eq!(pending.request("error".into(), 5, 5, || true), None);
        assert_eq!(
            pending.finish(Err("gone"), false),
            vec![("error".into(), InterruptOutcome::Unknown)]
        );
    }

    #[test]
    fn shutdown_marks_pending_interrupts_gone() {
        let mut pending = Cancellation::default();
        assert_eq!(pending.request("a".into(), 1, 1, || true), None);
        assert_eq!(
            pending.finish(Ok("cancelled"), true),
            vec![("a".into(), InterruptOutcome::Gone)]
        );
    }

    #[test]
    fn pending_receipts_are_bounded() {
        let mut pending = Cancellation::default();
        for i in 0..LIMIT {
            assert_eq!(pending.request(i.to_string(), 1, 1, || true), None);
        }
        assert_eq!(
            pending.request("overflow".into(), 1, 1, || false),
            Some(InterruptOutcome::Dropped)
        );
        assert_eq!(pending.finish(Ok("cancelled"), false).len(), LIMIT);
    }
}
