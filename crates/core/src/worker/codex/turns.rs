use crate::worker::acp::REPORT_TOOL;
use crate::worker::{
    stop_problem, turn_outcome, turn_stop, HarnessId, Inbound, PromptQueue, TurnEnd, TurnOutcome,
    WorkerEvent, REPORT_REMINDER, SERVER_NAME,
};
use serde_json::Value;

pub const TURN_COMPLETED: &str = "turn/completed";
pub const ITEM_COMPLETED: &str = "item/completed";

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Start(String),
    Emit(WorkerEvent),
    MarkUnreported,
    Snapshot,
    PostFailure(String),
}

#[derive(Debug, Default)]
pub struct Turns {
    queue: PromptQueue,
    running: Option<String>,
    reported: bool,
}

impl Turns {
    pub fn accept(&mut self, text: String) -> Vec<Action> {
        self.queue.accept(Inbound::Prompt(text));
        self.advance()
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
        let outcome = turn_outcome(
            turn_stop(&end),
            self.reported,
            text == REPORT_REMINDER,
            closing,
            false,
        );
        let mut actions = Vec::new();
        if !closing {
            actions.push(Action::Snapshot);
        }
        actions.push(Action::Emit(WorkerEvent::TurnEnded(end)));
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
}
