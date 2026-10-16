use super::{
    opencode_blocked_content, turn_stop, ApiData, PermissionAsked, SessionUpdate, TurnEnd,
    TurnResult,
};
use crate::worker::{turn_outcome, BlockedContent, TurnOutcome, REPORT_REMINDER};
use serde::Deserialize;
use std::collections::{HashMap, VecDeque};

pub const TRANSIENT_RETRIES: u32 = 2;
pub const SERVER_STOPPED: &str = "the opencode server stopped during the turn";

pub fn running_sessions(body: &str) -> Option<Vec<String>> {
    #[derive(Deserialize)]
    struct Activity {
        #[serde(rename = "type")]
        kind: String,
    }
    let active: ApiData<HashMap<String, Activity>> = serde_json::from_str(body).ok()?;
    let mut running: Vec<String> = active
        .data
        .into_iter()
        .filter(|(_, activity)| activity.kind == "running")
        .map(|(id, _)| id)
        .collect();
    running.sort();
    Some(running)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnState {
    pub in_flight: bool,
    pub reminded: bool,
    pub retries: u32,
    pub prompt: String,
    pub blocked: VecDeque<PermissionAsked>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnFacts {
    pub reported: bool,
    pub closing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Ready,
    ClearWaiting,
    Publish(Option<BlockedContent>),
    Send(String),
    ClearReported,
    Unreported,
    Problem(String),
}

impl TurnState {
    fn front(&self) -> Option<BlockedContent> {
        self.blocked.front().map(opencode_blocked_content)
    }

    pub fn prompted(&mut self, text: &str) {
        self.in_flight = true;
        self.reminded = false;
        self.retries = 0;
        self.prompt = text.to_string();
    }

    pub fn server_lost(&mut self) -> Vec<Effect> {
        let was_in_flight = self.in_flight;
        let was_blocked = !self.blocked.is_empty();
        *self = Self::default();
        let mut effects = Vec::new();
        if was_blocked {
            effects.push(Effect::Publish(None));
        }
        if was_in_flight {
            effects.push(Effect::Problem(SERVER_STOPPED.to_string()));
        }
        effects
    }

    fn end(&mut self, end: &TurnEnd, facts: TurnFacts) -> Vec<Effect> {
        self.in_flight = false;
        let retry = matches!(
            end,
            TurnEnd::Failed {
                transient: true,
                ..
            }
        ) && self.retries < TRANSIENT_RETRIES
            && !self.prompt.is_empty()
            && !facts.closing;
        if retry {
            self.retries += 1;
            self.in_flight = true;
            return vec![Effect::Send(self.prompt.clone())];
        }
        self.retries = 0;
        let outcome = turn_outcome(
            turn_stop(end),
            facts.reported,
            self.reminded,
            facts.closing,
            false,
        );
        self.reminded = outcome == TurnOutcome::Remind;
        match outcome {
            TurnOutcome::Remind => {
                self.in_flight = true;
                self.prompt = REPORT_REMINDER.to_string();
                vec![Effect::Send(self.prompt.clone())]
            }
            TurnOutcome::Unreported => vec![Effect::Unreported],
            TurnOutcome::Problem(problem) => vec![Effect::Problem(problem)],
            TurnOutcome::Cancelled => {
                self.blocked.clear();
                vec![Effect::Publish(None)]
            }
            TurnOutcome::Reported => vec![Effect::ClearReported],
            TurnOutcome::Closed => Vec::new(),
        }
    }
}

pub fn advance(state: &mut TurnState, update: SessionUpdate, facts: TurnFacts) -> Vec<Effect> {
    match update {
        SessionUpdate::TurnStarted => {
            state.in_flight = true;
            vec![Effect::Ready, Effect::ClearWaiting]
        }
        SessionUpdate::Blocked(asked) => {
            if state.blocked.iter().any(|known| known.id == asked.id) {
                return Vec::new();
            }
            state.blocked.push_back(asked);
            vec![Effect::Publish(state.front())]
        }
        SessionUpdate::Unblocked { request_id } => {
            let before = state.blocked.len();
            state.blocked.retain(|known| known.id != request_id);
            if state.blocked.len() == before {
                Vec::new()
            } else {
                vec![Effect::Publish(state.front())]
            }
        }
        SessionUpdate::TurnEnded(end) => state.end(&end, facts),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSnapshot {
    pub active: bool,
    pub pending: Vec<PermissionAsked>,
    pub result: Option<TurnResult>,
}

pub fn reconcile(state: &TurnState, snapshot: &SessionSnapshot) -> Vec<SessionUpdate> {
    let mut updates: Vec<SessionUpdate> = state
        .blocked
        .iter()
        .filter(|known| !snapshot.pending.iter().any(|asked| asked.id == known.id))
        .map(|known| SessionUpdate::Unblocked {
            request_id: known.id.clone(),
        })
        .collect();
    updates.extend(
        snapshot
            .pending
            .iter()
            .filter(|asked| !state.blocked.iter().any(|known| known.id == asked.id))
            .cloned()
            .map(SessionUpdate::Blocked),
    );
    match (state.in_flight, snapshot.active, &snapshot.result) {
        (false, true, _) => updates.push(SessionUpdate::TurnStarted),
        (true, false, Some(result)) => {
            updates.push(SessionUpdate::TurnEnded(result.end.clone()));
        }
        _ => {}
    }
    updates
}

#[cfg(test)]
mod tests {
    use super::super::tests::{asked, failed_end, messages, MESSAGES_OK, PERMISSION_ID};
    use super::super::turn_result;
    use super::*;

    fn facts() -> TurnFacts {
        TurnFacts {
            reported: false,
            closing: false,
        }
    }

    fn ended(state: &mut TurnState, end: TurnEnd, facts: TurnFacts) -> Vec<Effect> {
        advance(state, SessionUpdate::TurnEnded(end), facts)
    }

    #[test]
    fn running_sessions_reads_the_active_sample() {
        assert_eq!(
            running_sessions(r#"{"data":{"ses_ebb06fb5dffePK4O3B4WCKFy5g":{"type":"running"}}}"#),
            Some(vec!["ses_ebb06fb5dffePK4O3B4WCKFy5g".to_string()])
        );
        assert_eq!(running_sessions(r#"{"data":{}}"#), Some(Vec::new()));
        assert_eq!(
            running_sessions(r#"{"data":{"ses_a":{"type":"idle"}}}"#),
            Some(Vec::new())
        );
        assert_eq!(running_sessions("nope"), None);
    }

    #[test]
    fn prompted_starts_a_fresh_turn_that_can_retry_the_text() {
        let mut state = TurnState {
            reminded: true,
            retries: 2,
            prompt: "old".to_string(),
            ..TurnState::default()
        };
        state.prompted("new");
        assert!(state.in_flight);
        assert!(!state.reminded);
        assert_eq!(state.retries, 0);
        assert_eq!(state.prompt, "new");
        let effects = ended(&mut state, failed_end("503", true), facts());
        assert_eq!(effects, vec![Effect::Send("new".to_string())]);
    }

    #[test]
    fn turn_started_marks_ready_and_in_flight() {
        let mut state = TurnState::default();
        assert_eq!(
            advance(&mut state, SessionUpdate::TurnStarted, facts()),
            vec![Effect::Ready, Effect::ClearWaiting]
        );
        assert!(state.in_flight);
    }

    #[test]
    fn permissions_queue_in_order_and_publish_the_front() {
        let mut state = TurnState::default();
        let first = asked();
        let mut second = asked();
        second.id = "per_second".to_string();
        let front = opencode_blocked_content(&first);
        assert_eq!(
            advance(&mut state, SessionUpdate::Blocked(first.clone()), facts()),
            vec![Effect::Publish(Some(front.clone()))]
        );
        assert_eq!(
            advance(&mut state, SessionUpdate::Blocked(first.clone()), facts()),
            Vec::new()
        );
        assert_eq!(
            advance(&mut state, SessionUpdate::Blocked(second.clone()), facts()),
            vec![Effect::Publish(Some(front))]
        );
        assert_eq!(
            advance(
                &mut state,
                SessionUpdate::Unblocked {
                    request_id: PERMISSION_ID.to_string()
                },
                facts()
            ),
            vec![Effect::Publish(Some(opencode_blocked_content(&second)))]
        );
        assert_eq!(
            advance(
                &mut state,
                SessionUpdate::Unblocked {
                    request_id: "per_second".to_string()
                },
                facts()
            ),
            vec![Effect::Publish(None)]
        );
        assert_eq!(
            advance(
                &mut state,
                SessionUpdate::Unblocked {
                    request_id: "per_second".to_string()
                },
                facts()
            ),
            Vec::new()
        );
    }

    #[test]
    fn unreported_stop_reminds_once_then_waits() {
        let mut state = TurnState::default();
        assert_eq!(
            ended(&mut state, TurnEnd::Succeeded, facts()),
            vec![Effect::Send(REPORT_REMINDER.to_string())]
        );
        assert!(state.in_flight && state.reminded);
        assert_eq!(
            ended(&mut state, TurnEnd::Succeeded, facts()),
            vec![Effect::Unreported]
        );
        assert!(!state.in_flight && !state.reminded);
        assert_eq!(
            ended(&mut state, TurnEnd::Succeeded, facts()).len(),
            1,
            "a new turn is reminded again"
        );
    }

    #[test]
    fn reported_turn_clears_the_marker_and_the_reminder() {
        let mut state = TurnState {
            reminded: true,
            ..TurnState::default()
        };
        let reported = TurnFacts {
            reported: true,
            closing: false,
        };
        assert_eq!(
            ended(&mut state, TurnEnd::Succeeded, reported),
            vec![Effect::ClearReported]
        );
        assert!(!state.reminded);
    }

    #[test]
    fn closing_turn_does_nothing() {
        let mut state = TurnState::default();
        let closing = TurnFacts {
            reported: false,
            closing: true,
        };
        assert_eq!(ended(&mut state, TurnEnd::Succeeded, closing), Vec::new());
        assert_eq!(
            ended(&mut state, failed_end("x", true), closing),
            Vec::new()
        );
        assert!(!state.in_flight);
    }

    #[test]
    fn interrupted_turn_drops_pending_permissions() {
        let mut state = TurnState::default();
        advance(&mut state, SessionUpdate::Blocked(asked()), facts());
        assert_eq!(
            ended(&mut state, TurnEnd::Interrupted, facts()),
            vec![Effect::Publish(None)]
        );
        assert!(state.blocked.is_empty());
    }

    #[test]
    fn failed_turn_becomes_a_problem() {
        let mut state = TurnState::default();
        assert_eq!(
            ended(&mut state, failed_end("bad key", false), facts()),
            vec![Effect::Problem("bad key".to_string())]
        );
    }

    #[test]
    fn transient_failure_resends_the_prompt_twice_then_reports() {
        let mut state = TurnState {
            prompt: "do it".to_string(),
            ..TurnState::default()
        };
        let transient = failed_end("Not Found", true);
        for retry in 1..=TRANSIENT_RETRIES {
            assert_eq!(
                ended(&mut state, transient.clone(), facts()),
                vec![Effect::Send("do it".to_string())]
            );
            assert!(state.in_flight);
            assert_eq!(state.retries, retry);
        }
        assert_eq!(
            ended(&mut state, transient, facts()),
            vec![Effect::Problem("Not Found".to_string())]
        );
        assert_eq!(state.retries, 0);
    }

    #[test]
    fn transient_failure_without_a_prompt_is_a_problem() {
        let mut state = TurnState::default();
        assert_eq!(
            ended(&mut state, failed_end("Not Found", true), facts()),
            vec![Effect::Problem("Not Found".to_string())]
        );
    }

    #[test]
    fn a_finished_retry_resets_the_counter() {
        let mut state = TurnState {
            prompt: "do it".to_string(),
            ..TurnState::default()
        };
        ended(&mut state, failed_end("Not Found", true), facts());
        ended(&mut state, TurnEnd::Succeeded, facts());
        assert_eq!(state.retries, 0);
    }

    #[test]
    fn server_lost_reports_an_in_flight_turn_and_clears_state() {
        let mut state = TurnState::default();
        advance(&mut state, SessionUpdate::TurnStarted, facts());
        advance(&mut state, SessionUpdate::Blocked(asked()), facts());
        assert_eq!(
            state.server_lost(),
            vec![
                Effect::Publish(None),
                Effect::Problem(SERVER_STOPPED.to_string())
            ]
        );
        assert_eq!(state, TurnState::default());
        assert_eq!(state.server_lost(), Vec::new());
    }

    fn snapshot(active: bool) -> SessionSnapshot {
        SessionSnapshot {
            active,
            pending: Vec::new(),
            result: None,
        }
    }

    #[test]
    fn reconcile_finds_a_turn_started_while_disconnected() {
        assert_eq!(
            reconcile(&TurnState::default(), &snapshot(true)),
            vec![SessionUpdate::TurnStarted]
        );
        assert_eq!(
            reconcile(&TurnState::default(), &snapshot(false)),
            Vec::new()
        );
    }

    #[test]
    fn reconcile_ends_an_in_flight_turn_that_finished_while_disconnected() {
        let state = TurnState {
            in_flight: true,
            ..TurnState::default()
        };
        let done = SessionSnapshot {
            result: turn_result(&messages(MESSAGES_OK), false),
            ..snapshot(false)
        };
        assert_eq!(
            reconcile(&state, &done),
            vec![SessionUpdate::TurnEnded(TurnEnd::Succeeded)]
        );
        assert_eq!(reconcile(&state, &snapshot(false)), Vec::new());
        assert_eq!(reconcile(&state, &snapshot(true)), Vec::new());
    }

    #[test]
    fn reconcile_syncs_permissions_with_the_server() {
        let known = asked();
        let mut fresh = asked();
        fresh.id = "per_fresh".to_string();
        let state = TurnState {
            in_flight: true,
            blocked: VecDeque::from([known.clone()]),
            ..TurnState::default()
        };
        let snapshot = SessionSnapshot {
            pending: vec![fresh.clone()],
            ..snapshot(true)
        };
        assert_eq!(
            reconcile(&state, &snapshot),
            vec![
                SessionUpdate::Unblocked {
                    request_id: known.id
                },
                SessionUpdate::Blocked(fresh)
            ]
        );
    }
}
