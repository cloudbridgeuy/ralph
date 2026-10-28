use super::{MCP_STARTUP_STATUS, MODEL_REROUTED, SERVER_REQUEST_RESOLVED, TOKEN_USAGE_UPDATED};
use crate::worker::acp::REPORT_TOOL;
use crate::worker::{
    cancels_turn, stop_problem, turn_outcome, turn_stop, HarnessId, Inbound, PromptQueue, TurnEnd,
    TurnOutcome, WorkerEvent, REPORT_REMINDER, SERVER_NAME,
};
use serde_json::Value;

pub const TURN_COMPLETED: &str = "turn/completed";
pub const ITEM_COMPLETED: &str = "item/completed";
pub const TURN_STARTED: &str = "turn/started";
pub const NO_ACTIVE_TURN: i64 = -32600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadSignal {
    Reported,
    TurnEnded(TurnEnd),
}

fn decode_turn_end(turn: &Value) -> Option<TurnEnd> {
    match turn["status"].as_str()? {
        "completed" => Some(TurnEnd::Succeeded),
        "interrupted" => Some(TurnEnd::Interrupted),
        "failed" => Some(TurnEnd::Failed {
            message: turn["error"]["message"]
                .as_str()
                .unwrap_or("the turn failed")
                .to_string(),
            transient: false,
        }),
        _ => None,
    }
}

fn is_delivered_report(item: &Value) -> bool {
    item["type"] == "mcpToolCall"
        && item["server"] == SERVER_NAME
        && item["tool"] == REPORT_TOOL
        && item["status"] == "completed"
}

pub fn decode_signal(method: &str, params: &Value) -> Option<ThreadSignal> {
    match method {
        TURN_COMPLETED => decode_turn_end(&params["turn"]).map(ThreadSignal::TurnEnded),
        ITEM_COMPLETED => is_delivered_report(&params["item"]).then_some(ThreadSignal::Reported),
        _ => None,
    }
}

pub fn is_unmapped(method: &str) -> bool {
    !matches!(
        method,
        TURN_STARTED
            | TURN_COMPLETED
            | ITEM_COMPLETED
            | MCP_STARTUP_STATUS
            | SERVER_REQUEST_RESOLVED
            | TOKEN_USAGE_UPDATED
            | MODEL_REROUTED
    )
}

pub fn event_turn_id<'a>(method: &str, params: &'a Value) -> Option<&'a str> {
    match method {
        TURN_STARTED | TURN_COMPLETED => params["turn"]["id"].as_str(),
        _ => params["turnId"].as_str(),
    }
}

pub fn is_stale(current: Option<&str>, event: Option<&str>) -> bool {
    matches!((current, event), (Some(current), Some(event)) if current != event)
}

pub fn is_current(current: Option<&str>, event: Option<&str>) -> bool {
    matches!((current, event), (Some(current), Some(event)) if current == event)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Start(String),
    Interrupt(String),
    InterruptResult { entry: String, cancelled: bool },
    Emit(WorkerEvent),
    MarkUnreported,
    Snapshot,
    PostFailure(String),
}

#[derive(Debug, Default)]
pub struct Turns {
    queue: PromptQueue,
    running: Option<String>,
    turn_id: Option<String>,
    interrupts: Vec<String>,
    interrupt_sent: bool,
    close_requested: bool,
    reported: bool,
}

impl Turns {
    pub fn accept(&mut self, text: String) -> Vec<Action> {
        self.queue.accept(Inbound::Prompt(text));
        self.advance()
    }

    pub fn interrupt(&mut self, text: String, entry: String) -> Vec<Action> {
        let steps = self.queue.accept(Inbound::Interrupt(text, entry.clone()));
        if cancels_turn(&steps) {
            self.interrupts.push(entry);
            return self.send_interrupt();
        }
        let mut actions = vec![Action::InterruptResult {
            entry,
            cancelled: false,
        }];
        actions.extend(self.advance());
        actions
    }

    pub fn close(&mut self) -> Vec<Action> {
        if self.running.is_none() {
            return Vec::new();
        }
        self.close_requested = true;
        self.send_interrupt()
    }

    pub fn bind_turn(&mut self, turn_id: Option<String>) -> Vec<Action> {
        if self.running.is_none() {
            return Vec::new();
        }
        self.turn_id = turn_id;
        self.send_interrupt()
    }

    pub fn interrupt_failed(&mut self, code: i64) -> Vec<Action> {
        if code != NO_ACTIVE_TURN {
            return Vec::new();
        }
        self.resolve_interrupts(false)
    }

    pub fn is_stale_request(&self, method: &str, params: &Value) -> bool {
        is_stale(self.turn_id.as_deref(), event_turn_id(method, params))
    }

    pub fn owns_event(&self, method: &str, params: &Value) -> bool {
        is_current(self.turn_id.as_deref(), event_turn_id(method, params))
    }

    pub fn notification(&mut self, method: &str, params: &Value, closing: bool) -> Vec<Action> {
        let event = event_turn_id(method, params);
        if is_stale(self.turn_id.as_deref(), event) {
            return Vec::new();
        }
        if method == TURN_STARTED && self.turn_id.is_none() {
            return self.bind_turn(event.map(str::to_string));
        }
        decode_signal(method, params)
            .map(|signal| self.signal(signal, closing))
            .unwrap_or_default()
    }

    fn send_interrupt(&mut self) -> Vec<Action> {
        match &self.turn_id {
            Some(turn_id)
                if (!self.interrupts.is_empty() || self.close_requested)
                    && !self.interrupt_sent =>
            {
                self.interrupt_sent = true;
                vec![Action::Interrupt(turn_id.clone())]
            }
            _ => Vec::new(),
        }
    }

    fn resolve_interrupts(&mut self, cancelled: bool) -> Vec<Action> {
        std::mem::take(&mut self.interrupts)
            .into_iter()
            .map(|entry| Action::InterruptResult { entry, cancelled })
            .collect()
    }

    fn advance(&mut self) -> Vec<Action> {
        if self.running.is_some() {
            return Vec::new();
        }
        let Some(text) = self.queue.start() else {
            return Vec::new();
        };
        self.reported = false;
        self.running = Some(text.clone());
        self.turn_id = None;
        self.interrupt_sent = false;
        vec![Action::Emit(WorkerEvent::TurnStarted), Action::Start(text)]
    }

    pub fn signal(&mut self, signal: ThreadSignal, closing: bool) -> Vec<Action> {
        match signal {
            ThreadSignal::Reported if self.running.is_some() && !self.reported => {
                self.reported = true;
                vec![Action::Emit(WorkerEvent::Reported)]
            }
            ThreadSignal::TurnEnded(end) => self.end(end, closing),
            ThreadSignal::Reported => Vec::new(),
        }
    }

    fn end(&mut self, end: TurnEnd, closing: bool) -> Vec<Action> {
        let Some(text) = self.running.take() else {
            return Vec::new();
        };
        self.queue.finish();
        let cancelled = end == TurnEnd::Interrupted;
        let outcome = turn_outcome(
            turn_stop(&end),
            self.reported,
            text == REPORT_REMINDER,
            closing,
            cancelled,
        );
        let mut actions = Vec::new();
        if !closing {
            actions.push(Action::Snapshot);
        }
        actions.push(Action::Emit(WorkerEvent::TurnEnded(end)));
        actions.extend(self.resolve_interrupts(cancelled));
        match outcome {
            TurnOutcome::Remind => {
                self.queue
                    .accept(Inbound::Prompt(REPORT_REMINDER.to_string()));
            }
            TurnOutcome::Unreported => actions.push(Action::MarkUnreported),
            TurnOutcome::Problem(problem) => actions.push(Action::PostFailure(problem)),
            TurnOutcome::Stopped(reason) => actions.push(Action::PostFailure(stop_problem(
                HarnessId::Codex.as_str(),
                &reason,
            ))),
            TurnOutcome::Reported | TurnOutcome::Cancelled | TurnOutcome::Closed => {}
        }
        actions.extend(self.advance());
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::decode_turn_id;
    use serde_json::json;

    fn item(server: &str, tool: &str, status: &str) -> Value {
        json!({"threadId": "t", "turnId": "u", "item": {"type": "mcpToolCall", "id": "i", "server": server, "tool": tool, "status": status, "arguments": {}}})
    }

    fn done() -> ThreadSignal {
        ThreadSignal::TurnEnded(TurnEnd::Succeeded)
    }

    fn starts(actions: &[Action]) -> Vec<&str> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::Start(text) => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn only_a_completed_report_call_counts_as_delivered() {
        let decode = |p: Value| decode_signal(ITEM_COMPLETED, &p);
        assert_eq!(
            decode(item("ralph-worker", "report", "completed")),
            Some(ThreadSignal::Reported)
        );
        for p in [
            item("ralph-worker", "report", "failed"),
            item("ralph-worker", "report", "inProgress"),
            item("ralph-worker", "other", "completed"),
            item("mcptools", "report", "completed"),
            json!({"item": {"type": "commandExecution", "status": "completed"}}),
        ] {
            assert_eq!(decode(p), None);
        }
        assert_eq!(
            decode_signal("item/started", &item("ralph-worker", "report", "completed")),
            None
        );
    }

    #[test]
    fn turn_completed_maps_each_status() {
        let decode =
            |turn: Value| decode_signal(TURN_COMPLETED, &json!({"threadId": "t", "turn": turn}));
        assert_eq!(
            decode(json!({"id": "u", "status": "completed", "items": []})),
            Some(done())
        );
        assert_eq!(
            decode(json!({"id": "u", "status": "interrupted", "items": []})),
            Some(ThreadSignal::TurnEnded(TurnEnd::Interrupted))
        );
        assert_eq!(
            decode(
                json!({"id": "u", "status": "failed", "items": [], "error": {"message": "boom"}})
            ),
            Some(ThreadSignal::TurnEnded(TurnEnd::Failed {
                message: "boom".into(),
                transient: false
            }))
        );
        assert_eq!(
            decode(json!({"id": "u", "status": "inProgress", "items": []})),
            None
        );
    }

    #[test]
    fn closing_interrupts_the_running_turn_once_its_id_is_bound() {
        let mut turns = Turns::default();
        assert!(turns.close().is_empty());
        turns.accept("task".into());
        assert!(turns.close().is_empty());
        assert_eq!(
            turns.bind_turn(Some("u".into())),
            vec![Action::Interrupt("u".into())]
        );
        assert!(turns.close().is_empty());
    }

    #[test]
    fn a_reply_during_a_turn_waits_for_the_turn_to_end() {
        let mut turns = Turns::default();
        assert_eq!(starts(&turns.accept("task".into())), ["task"]);
        assert!(turns.accept("one".into()).is_empty());
        assert!(turns.accept("two".into()).is_empty());
        turns.signal(ThreadSignal::Reported, false);
        assert_eq!(starts(&turns.signal(done(), false)), ["one"]);
        turns.signal(ThreadSignal::Reported, false);
        assert_eq!(starts(&turns.signal(done(), false)), ["two"]);
    }

    #[test]
    fn an_unreported_turn_gets_one_reminder_then_is_marked_unreported() {
        let mut turns = Turns::default();
        turns.accept("task".into());
        let first = turns.signal(done(), false);
        assert_eq!(starts(&first), [REPORT_REMINDER]);
        assert!(!first.contains(&Action::MarkUnreported));
        let second = turns.signal(done(), false);
        assert!(second.contains(&Action::MarkUnreported));
        assert!(starts(&second).is_empty());
    }

    #[test]
    fn a_failed_turn_posts_its_error_and_still_snapshots_first() {
        let mut turns = Turns::default();
        turns.accept("task".into());
        let failed = ThreadSignal::TurnEnded(TurnEnd::Failed {
            message: "boom".into(),
            transient: false,
        });
        let actions = turns.signal(failed, false);
        assert_eq!(actions[0], Action::Snapshot);
        assert!(actions.contains(&Action::PostFailure("boom".into())));
    }

    #[test]
    fn an_interrupted_turn_posts_nothing_and_closing_skips_the_snapshot() {
        let mut turns = Turns::default();
        turns.accept("task".into());
        let actions = turns.signal(ThreadSignal::TurnEnded(TurnEnd::Interrupted), false);
        assert!(!actions.iter().any(|a| matches!(a, Action::PostFailure(_))));
        turns.accept("again".into());
        let closing = turns.signal(done(), true);
        assert!(!closing.contains(&Action::Snapshot));
    }

    #[test]
    fn a_snapshot_comes_before_the_next_queued_start() {
        let mut turns = Turns::default();
        turns.accept("task".into());
        turns.accept("next".into());
        turns.signal(ThreadSignal::Reported, false);
        let actions = turns.signal(done(), false);
        let snapshot = actions.iter().position(|a| *a == Action::Snapshot);
        let start = actions.iter().position(|a| matches!(a, Action::Start(_)));
        assert!(snapshot < start);
    }

    #[test]
    fn a_report_emits_once_and_skips_the_reminder() {
        let mut turns = Turns::default();
        turns.accept("task".into());
        assert_eq!(
            turns.signal(ThreadSignal::Reported, false),
            [Action::Emit(WorkerEvent::Reported)]
        );
        assert!(turns.signal(ThreadSignal::Reported, false).is_empty());
        let ended = turns.signal(done(), false);
        assert!(starts(&ended).is_empty() && !ended.contains(&Action::MarkUnreported));
        assert!(turns.signal(ThreadSignal::Reported, false).is_empty());
    }

    fn completed(turn: &str, status: &str) -> Value {
        json!({"threadId": "t", "turn": {"id": turn, "status": status, "items": []}})
    }

    fn results(actions: &[Action]) -> Vec<(&str, bool)> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::InterruptResult { entry, cancelled } => Some((entry.as_str(), *cancelled)),
                _ => None,
            })
            .collect()
    }

    fn running(text: &str, turn: &str) -> Turns {
        let mut turns = Turns::default();
        turns.accept(text.into());
        turns.bind_turn(Some(turn.into()));
        turns
    }

    #[test]
    fn a_running_interrupt_sends_turn_interrupt_then_cancels_on_the_matching_completion() {
        let mut turns = running("task", "u1");
        turns.accept("queued".into());
        let sent = turns.interrupt("now".into(), "e1".into());
        assert_eq!(sent, [Action::Interrupt("u1".into())]);
        let ended = turns.notification(TURN_COMPLETED, &completed("u1", "interrupted"), false);
        assert_eq!(results(&ended), [("e1", true)]);
        assert_eq!(starts(&ended), ["now"]);
        assert!(!ended.contains(&Action::MarkUnreported));
        assert!(!ended.iter().any(|a| matches!(a, Action::PostFailure(_))));
        turns.bind_turn(Some("u2".into()));
        turns.signal(ThreadSignal::Reported, false);
        let next = turns.notification(TURN_COMPLETED, &completed("u2", "completed"), false);
        assert_eq!(starts(&next), ["queued"]);
    }

    #[test]
    fn an_idle_interrupt_reports_nothing_cancelled_and_sends_no_turn_interrupt() {
        let mut turns = Turns::default();
        let actions = turns.interrupt("now".into(), "e1".into());
        assert_eq!(results(&actions), [("e1", false)]);
        assert_eq!(starts(&actions), ["now"]);
        assert!(!actions.iter().any(|a| matches!(a, Action::Interrupt(_))));
    }

    #[test]
    fn no_active_turn_error_counts_as_idle_and_other_errors_leave_the_outcome_unknown() {
        let mut turns = running("task", "u1");
        turns.interrupt("now".into(), "e1".into());
        assert!(turns.interrupt_failed(-32000).is_empty());
        assert_eq!(
            results(&turns.interrupt_failed(NO_ACTIVE_TURN)),
            [("e1", false)]
        );
    }

    #[test]
    fn a_refused_interrupt_while_running_leaves_the_turn_to_end_normally() {
        let mut turns = running("task", "u1");
        turns.interrupt("now".into(), "e1".into());
        let refused = turns.interrupt_failed(NO_ACTIVE_TURN);
        assert_eq!(results(&refused), [("e1", false)]);
        assert!(starts(&refused).is_empty());
        let ended = turns.notification(TURN_COMPLETED, &completed("u1", "completed"), false);
        assert!(results(&ended).is_empty());
        assert_eq!(starts(&ended), ["now"]);
        turns.bind_turn(Some("u2".into()));
        turns.signal(ThreadSignal::Reported, false);
        let next = turns.notification(TURN_COMPLETED, &completed("u2", "completed"), false);
        assert_eq!(starts(&next), [REPORT_REMINDER]);
    }

    #[test]
    fn a_missing_interrupt_reply_holds_the_prompt_until_the_turn_ends() {
        let mut turns = running("task", "u1");
        turns.interrupt("now".into(), "e1".into());
        let late_item = turns.notification(
            ITEM_COMPLETED,
            &item("ralph-worker", "report", "completed"),
            false,
        );
        assert!(late_item.is_empty());
        let ended = turns.notification(TURN_COMPLETED, &completed("u1", "completed"), false);
        assert_eq!(results(&ended), [("e1", false)]);
        assert_eq!(starts(&ended), ["now"]);
    }

    #[test]
    fn only_methods_the_worker_consumes_are_mapped() {
        for method in [
            TURN_STARTED,
            TURN_COMPLETED,
            ITEM_COMPLETED,
            MCP_STARTUP_STATUS,
            SERVER_REQUEST_RESOLVED,
            TOKEN_USAGE_UPDATED,
            MODEL_REROUTED,
        ] {
            assert!(!is_unmapped(method));
        }
        assert!(is_unmapped("item/agentMessage/delta"));
    }

    #[test]
    fn events_of_another_turn_are_dropped() {
        let mut turns = running("task", "new");
        let old_report = json!({"threadId": "t", "turnId": "old", "item": {"type": "mcpToolCall", "server": "ralph-worker", "tool": "report", "status": "completed"}});
        assert!(turns
            .notification(ITEM_COMPLETED, &old_report, false)
            .is_empty());
        assert!(turns
            .notification(TURN_COMPLETED, &completed("old", "completed"), false)
            .is_empty());
        let report = json!({"threadId": "t", "turnId": "new", "item": {"type": "mcpToolCall", "server": "ralph-worker", "tool": "report", "status": "completed"}});
        assert_eq!(
            turns.notification(ITEM_COMPLETED, &report, false),
            [Action::Emit(WorkerEvent::Reported)]
        );
        assert!(is_stale(Some("a"), Some("b")));
        assert!(!is_stale(None, Some("b")));
    }

    #[test]
    fn only_an_event_of_the_bound_turn_is_current() {
        assert!(is_current(Some("a"), Some("a")));
        assert!(!is_current(Some("a"), Some("b")));
        assert!(!is_current(None, Some("a")));
        assert!(!is_current(Some("a"), None));
        assert!(!is_current(None, None));
    }

    #[test]
    fn usage_belongs_to_the_bound_turn_only() {
        let usage = |turn: &str| json!({"threadId": "t", "turnId": turn, "tokenUsage": {}});
        let mut turns = Turns::default();
        assert!(!turns.owns_event(TOKEN_USAGE_UPDATED, &usage("u1")));
        turns.accept("task".into());
        turns.bind_turn(decode_turn_id(&json!({"turn": {"id": "u1"}})));
        assert!(turns.owns_event(TOKEN_USAGE_UPDATED, &usage("u1")));
        assert!(!turns.owns_event(TOKEN_USAGE_UPDATED, &usage("u0")));
        turns.notification(TURN_COMPLETED, &completed("u1", "completed"), false);
        assert!(!turns.owns_event(TOKEN_USAGE_UPDATED, &usage("u1")));
    }

    #[test]
    fn a_completion_buffered_before_the_start_response_still_ends_the_turn() {
        let mut turns = Turns::default();
        turns.accept("task".into());
        turns.bind_turn(decode_turn_id(&json!({"turn": {"id": "u1"}})));
        let ended = turns.notification(TURN_COMPLETED, &completed("u1", "completed"), false);
        assert!(ended.contains(&Action::Emit(WorkerEvent::TurnEnded(TurnEnd::Succeeded))));
        assert_eq!(starts(&ended), [REPORT_REMINDER]);
    }

    #[test]
    fn an_interrupt_before_the_turn_id_is_known_is_sent_once_it_binds() {
        let mut turns = Turns::default();
        turns.accept("task".into());
        assert!(turns.interrupt("now".into(), "e1".into()).is_empty());
        let started = json!({"threadId": "t", "turn": {"id": "u1"}});
        assert_eq!(
            turns.notification(TURN_STARTED, &started, false),
            [Action::Interrupt("u1".into())]
        );
    }
}
