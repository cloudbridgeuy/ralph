use crate::worker::{worker_prompt, Report, ReportStatus, REPORT_REMINDER};
use std::collections::VecDeque;

pub fn prompt_entry_name(nanos: u64, pid: u32) -> String {
    format!("{nanos:020}-{pid}")
}

pub fn task_prompt(task: &str, scheduled: bool) -> String {
    format!("{}\n\n{task}", worker_prompt(scheduled))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnOutcome {
    Reported,
    Remind,
    Unreported,
    Problem(String),
    Stopped(String),
    Closed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inbound {
    Prompt(String),
    Interrupt(String, String),
    TrackedPrompt(String, String),
    TrackedInterrupt(String, String, String, u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    CancelPermissions,
    CancelSession,
}

#[derive(Debug, Default)]
pub struct PromptQueue {
    pending: VecDeque<(String, Option<String>)>,
    running: bool,
    interrupted: bool,
    ordered: usize,
}

impl PromptQueue {
    pub fn accept(&mut self, inbound: Inbound) -> Vec<Step> {
        match inbound {
            Inbound::Prompt(text) => {
                self.pending.push_back((text, None));
                Vec::new()
            }
            Inbound::TrackedPrompt(text, receipt) => {
                self.pending.push_back((text, Some(receipt)));
                Vec::new()
            }
            Inbound::TrackedInterrupt(text, entry, receipt, _) => {
                let steps = self.accept(Inbound::Interrupt(text, entry));
                if let Some((_, slot)) = self.pending.front_mut() {
                    *slot = Some(receipt);
                }
                steps
            }
            Inbound::Interrupt(text, _) => {
                self.pending.push_front((text, None));
                if self.running {
                    self.interrupted = true;
                    vec![Step::CancelPermissions, Step::CancelSession]
                } else {
                    vec![Step::CancelPermissions]
                }
            }
        }
    }

    pub fn accept_ordered_interrupt(&mut self, text: String, receipt: String) {
        self.pending.insert(self.ordered, (text, Some(receipt)));
        self.ordered += 1;
    }

    pub fn remind(&mut self) {
        if !self.pending.iter().any(|(text, _)| text == REPORT_REMINDER) {
            self.pending.push_back((REPORT_REMINDER.to_string(), None));
        }
    }

    pub fn start_tracked(&mut self) -> Option<(String, Option<String>)> {
        let text = self.pending.pop_front()?;
        self.ordered = self.ordered.saturating_sub(1);
        self.running = true;
        self.interrupted = false;
        Some(text)
    }

    pub fn start(&mut self) -> Option<String> {
        self.start_tracked().map(|(text, _)| text)
    }

    pub fn finish(&mut self) -> bool {
        self.running = false;
        self.interrupted
    }
}

pub fn cancels_turn(steps: &[Step]) -> bool {
    steps.contains(&Step::CancelSession)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptOutcome {
    Cancelled,
    Sent,
    Gone,
    Dropped,
    Completed,
    Unknown,
}

impl From<bool> for InterruptOutcome {
    fn from(cancelled: bool) -> Self {
        if cancelled {
            Self::Cancelled
        } else {
            Self::Sent
        }
    }
}

pub fn interrupt_result_text(outcome: InterruptOutcome) -> &'static str {
    match outcome {
        InterruptOutcome::Cancelled => "cancelled",
        InterruptOutcome::Sent => "sent",
        InterruptOutcome::Gone => "gone",
        InterruptOutcome::Dropped => "dropped",
        InterruptOutcome::Completed => "completed",
        InterruptOutcome::Unknown => "unknown",
    }
}

pub fn parse_interrupt_result(text: &str) -> Option<InterruptOutcome> {
    match text.trim() {
        "cancelled" => Some(InterruptOutcome::Cancelled),
        "sent" => Some(InterruptOutcome::Sent),
        "gone" => Some(InterruptOutcome::Gone),
        "dropped" => Some(InterruptOutcome::Dropped),
        "completed" => Some(InterruptOutcome::Completed),
        "unknown" => Some(InterruptOutcome::Unknown),
        _ => None,
    }
}

pub fn interrupt_reply(id: &str, outcome: Option<InterruptOutcome>) -> String {
    match outcome {
        Some(InterruptOutcome::Cancelled) => format!(
            "Interrupted worker {id}: its running turn was cancelled and this message runs next."
        ),
        Some(InterruptOutcome::Sent) => {
            format!("Sent to worker {id}: no turn was running, so nothing was cancelled.")
        }
        Some(InterruptOutcome::Gone) => {
            format!("Worker {id} exited: its turn ended and this message was not delivered.")
        }
        Some(InterruptOutcome::Completed) => format!("Worker {id}: the prior turn ended before cancellation was confirmed; this message runs next."),
        Some(InterruptOutcome::Unknown) => format!("Worker {id}: cancellation outcome unknown; this message is queued."),
        Some(InterruptOutcome::Dropped) => format!(
            "Worker {id}: the interrupt was not sent and this message runs next."
        ),
        None => format!(
            "Interrupt queued for worker {id}, but ralph chat did not confirm within 30 s: outcome unknown."
        ),
    }
}

pub fn turn_outcome(
    stop: Result<&str, &str>,
    reported: bool,
    reminded: bool,
    closing: bool,
    interrupted: bool,
) -> TurnOutcome {
    if closing {
        return TurnOutcome::Closed;
    }
    if reported {
        return TurnOutcome::Reported;
    }
    match stop {
        Err(message) => TurnOutcome::Problem(message.to_string()),
        Ok("cancelled" | "end_turn") if interrupted => TurnOutcome::Cancelled,
        Ok("cancelled" | "end_turn") if reminded => TurnOutcome::Unreported,
        Ok("cancelled" | "end_turn") => TurnOutcome::Remind,
        Ok(other) => TurnOutcome::Stopped(other.to_string()),
    }
}

pub fn sweeps_permissions(stop: Result<&str, &str>, outcome: &TurnOutcome) -> bool {
    stop == Ok("cancelled") || *outcome == TurnOutcome::Cancelled
}

pub fn stop_problem(agent: &str, reason: &str) -> String {
    format!("the {agent} agent ended its turn with `{reason}`")
}

pub fn prompt_failure_report(agent: &str, id: &str, at: &str, problem: &str) -> Report {
    Report {
        worker: id.to_string(),
        at: at.to_string(),
        status: ReportStatus::Blocked.as_str().to_string(),
        text: format!("The {agent} task prompt failed: {problem}."),
    }
}

pub const REPEAT_REPORT_REFUSAL: &str = "Already reported this turn. End your turn now.";

pub const ONCE_PER_TURN_FLAG: &str = "--once-per-turn";

pub fn refuses_repeat_report(once_per_turn: bool, reported_this_turn: bool) -> bool {
    once_per_turn && reported_this_turn
}

pub fn accept_report(owner_required: bool, owner_exists: bool) -> bool {
    !owner_required || owner_exists
}

#[cfg(test)]
mod tests {
    use super::super::CURSOR_AGENT;
    use super::*;
    use crate::worker::{SCHEDULED_WORKER_PROMPT, WORKER_PROMPT};

    #[test]
    fn only_a_flagged_server_refuses_a_repeat_report() {
        assert!(refuses_repeat_report(true, true));
        assert!(!refuses_repeat_report(true, false));
        assert!(!refuses_repeat_report(false, true));
    }

    #[test]
    fn remind_queues_at_most_one_reminder() {
        let mut queue = PromptQueue::default();
        queue.accept(Inbound::Prompt("next".into()));
        queue.remind();
        queue.remind();
        assert_eq!(queue.start().as_deref(), Some("next"));
        assert_eq!(queue.start().as_deref(), Some(REPORT_REMINDER));
        assert_eq!(queue.start(), None);
    }

    #[test]
    fn an_unasked_cancel_sweeps_permissions() {
        assert!(sweeps_permissions(Ok("cancelled"), &TurnOutcome::Remind));
        assert!(sweeps_permissions(Ok("end_turn"), &TurnOutcome::Cancelled));
        assert!(!sweeps_permissions(Ok("end_turn"), &TurnOutcome::Remind));
        assert!(!sweeps_permissions(
            Err("boom"),
            &TurnOutcome::Problem("boom".into())
        ));
    }

    #[test]
    fn prompt_entries_sort_in_arrival_order() {
        assert!(prompt_entry_name(9, 5) < prompt_entry_name(10, 1));
    }

    #[test]
    fn a_report_needs_the_owner_file_only_when_one_is_required() {
        assert!(accept_report(false, false));
        assert!(accept_report(true, true));
        assert!(!accept_report(true, false));
    }

    #[test]
    fn task_prompt_is_the_worker_rules_then_the_task() {
        let prompt = task_prompt("do it", false);
        assert!(prompt.starts_with(WORKER_PROMPT));
        assert!(prompt.ends_with("\n\ndo it"));
    }

    #[test]
    fn scheduled_task_prompt_is_the_scheduled_rules_then_the_task() {
        let prompt = task_prompt("do it", true);
        assert!(prompt.starts_with(SCHEDULED_WORKER_PROMPT));
        assert!(prompt.ends_with("\n\ndo it"));
        assert!(!prompt.contains(WORKER_PROMPT));
    }

    #[test]
    fn turn_outcome_covers_every_branch() {
        let end = Ok("end_turn");
        assert_eq!(
            turn_outcome(end, true, false, false, false),
            TurnOutcome::Reported
        );
        assert_eq!(
            turn_outcome(end, false, false, false, false),
            TurnOutcome::Remind
        );
        assert_eq!(
            turn_outcome(end, false, true, false, false),
            TurnOutcome::Unreported
        );
        assert_eq!(
            turn_outcome(Ok("cancelled"), false, false, true, false),
            TurnOutcome::Closed
        );
        assert_eq!(
            turn_outcome(Err("x"), true, false, true, false),
            TurnOutcome::Closed
        );
        assert_eq!(
            turn_outcome(Ok("refusal"), false, false, false, false),
            TurnOutcome::Stopped("refusal".into())
        );
        assert_eq!(
            turn_outcome(Err("boom"), false, false, false, false),
            TurnOutcome::Problem("boom".into())
        );
        assert_eq!(
            turn_outcome(Ok("cancelled"), false, false, false, false),
            TurnOutcome::Remind
        );
        assert_eq!(
            turn_outcome(Ok("cancelled"), false, true, false, false),
            TurnOutcome::Unreported
        );
        assert_eq!(
            turn_outcome(Ok("cancelled"), false, true, false, true),
            TurnOutcome::Cancelled
        );
        assert_eq!(
            turn_outcome(end, false, false, false, true),
            TurnOutcome::Cancelled
        );
        assert_eq!(
            turn_outcome(Ok("cancelled"), true, false, false, true),
            TurnOutcome::Reported
        );
        assert_eq!(
            turn_outcome(end, false, true, false, true),
            TurnOutcome::Cancelled
        );
    }

    fn prompt(text: &str) -> Inbound {
        Inbound::Prompt(text.into())
    }

    fn interrupt(text: &str) -> Inbound {
        Inbound::Interrupt(text.into(), "entry".into())
    }

    #[test]
    fn interrupt_prompt_runs_before_queued_replies() {
        let mut queue = PromptQueue::default();
        queue.accept(prompt("a"));
        queue.accept(prompt("b"));
        queue.accept(interrupt("now"));
        assert_eq!(queue.start().as_deref(), Some("now"));
        queue.finish();
        assert_eq!(queue.start().as_deref(), Some("a"));
        queue.finish();
        assert_eq!(queue.start().as_deref(), Some("b"));
    }

    #[test]
    fn interrupt_while_idle_only_cancels_permissions() {
        let mut queue = PromptQueue::default();
        queue.accept(prompt("a"));
        assert_eq!(
            queue.accept(interrupt("now")),
            vec![Step::CancelPermissions]
        );
        assert_eq!(queue.start().as_deref(), Some("now"));
        assert!(!queue.finish());
    }

    #[test]
    fn interrupt_while_running_cancels_permissions_then_session() {
        let mut queue = PromptQueue::default();
        queue.accept(prompt("a"));
        queue.start();
        assert_eq!(
            queue.accept(interrupt("now")),
            vec![Step::CancelPermissions, Step::CancelSession]
        );
    }

    #[test]
    fn interrupt_reply_reports_each_outcome() {
        assert_eq!(
            interrupt_reply("GUZ-1", Some(InterruptOutcome::Cancelled)),
            "Interrupted worker GUZ-1: its running turn was cancelled and this message runs next."
        );
        assert_eq!(
            interrupt_reply("GUZ-1", Some(InterruptOutcome::Sent)),
            "Sent to worker GUZ-1: no turn was running, so nothing was cancelled."
        );
        assert_eq!(
            interrupt_reply("GUZ-1", Some(InterruptOutcome::Gone)),
            "Worker GUZ-1 exited: its turn ended and this message was not delivered."
        );
        assert_eq!(
            interrupt_reply("GUZ-1", Some(InterruptOutcome::Dropped)),
            "Worker GUZ-1: the interrupt was not sent and this message runs next."
        );
        assert_eq!(
            interrupt_reply("GUZ-1", None),
            "Interrupt queued for worker GUZ-1, but ralph chat did not confirm within 30 s: outcome unknown."
        );
    }

    #[test]
    fn cancels_turn_only_with_cancel_session() {
        assert!(cancels_turn(&[
            Step::CancelPermissions,
            Step::CancelSession
        ]));
        assert!(!cancels_turn(&[Step::CancelPermissions]));
    }

    #[test]
    fn ordered_interrupts_keep_receipts_and_precede_prompts() {
        let mut queue = PromptQueue::default();
        queue.accept(Inbound::Prompt("ordinary".into()));
        queue.accept_ordered_interrupt("first".into(), "one".into());
        queue.accept_ordered_interrupt("second".into(), "two".into());
        assert_eq!(
            queue.start_tracked(),
            Some(("first".into(), Some("one".into())))
        );
        queue.finish();
        assert_eq!(
            queue.start_tracked(),
            Some(("second".into(), Some("two".into())))
        );
        queue.finish();
        assert_eq!(queue.start_tracked(), Some(("ordinary".into(), None)));
    }

    #[test]
    fn interrupt_result_text_round_trips() {
        for outcome in [
            InterruptOutcome::Cancelled,
            InterruptOutcome::Sent,
            InterruptOutcome::Gone,
            InterruptOutcome::Dropped,
            InterruptOutcome::Completed,
            InterruptOutcome::Unknown,
        ] {
            assert_eq!(
                parse_interrupt_result(interrupt_result_text(outcome)),
                Some(outcome)
            );
        }
        assert_eq!(parse_interrupt_result(""), None);
    }

    #[test]
    fn interrupted_turn_is_cancelled_and_the_interrupt_starts_once() {
        let mut queue = PromptQueue::default();
        queue.accept(prompt("a"));
        queue.start();
        queue.accept(interrupt("now"));
        let interrupted = queue.finish();
        assert!(interrupted);
        assert_eq!(
            turn_outcome(Ok("end_turn"), false, false, false, interrupted),
            TurnOutcome::Cancelled
        );
        assert_eq!(queue.start().as_deref(), Some("now"));
        assert!(!queue.finish());
        assert_eq!(queue.start(), None);
    }

    #[test]
    fn prompt_failure_report_blocks_the_worker() {
        let report = prompt_failure_report(CURSOR_AGENT.name, "A", "2026-01-01T00:00:00Z", "boom");
        assert_eq!(report.worker, "A");
        assert_eq!(report.status, "blocked");
        assert!(report.text.contains("boom"));
    }

    #[test]
    fn a_cursor_stop_reason_keeps_its_exact_failure_text() {
        let report = prompt_failure_report(
            CURSOR_AGENT.name,
            "A",
            "2026-01-01T00:00:00Z",
            &stop_problem(CURSOR_AGENT.name, "refusal"),
        );
        assert_eq!(
            report.text,
            "The cursor task prompt failed: the cursor agent ended its turn with `refusal`."
        );
    }
}
