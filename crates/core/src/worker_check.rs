use crate::worker::{startup_state, Startup};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerObservation {
    pub id: String,
    pub session: String,
    pub ready: bool,
    pub blocked: bool,
    pub waiting: bool,
    pub reported_since_nudge: bool,
    pub pane: Option<String>,
    pub pane_changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerState {
    Working,
    Waiting,
    BlockedPermission,
    BlockedTrust,
    Unknown,
    Reported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckAction {
    Nudge(String),
    Inbox(String, String),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CheckMemory {
    pub workers: BTreeMap<String, WorkerMemory>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerMemory {
    pub state: WorkerState,
    pub nudged: bool,
}

pub fn classify(obs: &WorkerObservation) -> WorkerState {
    if obs.blocked {
        return WorkerState::BlockedPermission;
    }
    if let Some(pane) = &obs.pane {
        if startup_state(pane, obs.ready) == Startup::TrustPrompt {
            return WorkerState::BlockedTrust;
        }
    }
    if obs.waiting {
        WorkerState::Waiting
    } else if obs.reported_since_nudge {
        WorkerState::Reported
    } else if obs.pane.is_some() && !obs.pane_changed {
        WorkerState::Unknown
    } else {
        WorkerState::Working
    }
}

fn inbox_text(state: WorkerState, obs: &WorkerObservation) -> Option<String> {
    let id = &obs.id;
    let session = &obs.session;
    match state {
        WorkerState::Waiting => Some(format!(
            "Worker {id} stopped without calling report. To look: tmux attach -t {session}"
        )),
        WorkerState::BlockedPermission => Some(format!(
            "Worker {id} is waiting at a permission prompt; ralph does not type into it. To answer: tmux attach -t {session}"
        )),
        WorkerState::BlockedTrust => Some(format!(
            "Worker {id} is waiting at the folder-trust dialog. To answer: tmux attach -t {session}"
        )),
        WorkerState::Unknown => Some(format!(
            "ralph can't tell what worker {id} is waiting on: its pane has not changed and it has not reported. To look: tmux attach -t {session}"
        )),
        WorkerState::Working | WorkerState::Reported => None,
    }
}

pub fn plan(
    prev: &CheckMemory,
    observations: &[WorkerObservation],
) -> (Vec<CheckAction>, CheckMemory) {
    let mut actions = Vec::new();
    let mut next = CheckMemory::default();
    for obs in observations {
        let state = classify(obs);
        let prev_entry = prev.workers.get(&obs.id);
        let mut nudged = !obs.reported_since_nudge && prev_entry.is_some_and(|p| p.nudged);
        if state == WorkerState::Waiting && !nudged {
            actions.push(CheckAction::Nudge(obs.id.clone()));
            nudged = true;
        }
        if prev_entry.map(|p| p.state) != Some(state) {
            if let Some(text) = inbox_text(state, obs) {
                actions.push(CheckAction::Inbox(obs.id.clone(), text));
            }
        }
        next.workers
            .insert(obs.id.clone(), WorkerMemory { state, nudged });
    }
    (actions, next)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs() -> WorkerObservation {
        WorkerObservation {
            id: "w1".to_string(),
            session: "sess".to_string(),
            ready: true,
            blocked: false,
            waiting: false,
            reported_since_nudge: false,
            pane: None,
            pane_changed: true,
        }
    }

    fn waiting() -> WorkerObservation {
        WorkerObservation {
            waiting: true,
            ..obs()
        }
    }

    fn trust_pane() -> Option<String> {
        Some("Yes, I trust this folder".to_string())
    }

    fn nudges(actions: &[CheckAction]) -> usize {
        actions
            .iter()
            .filter(|a| matches!(a, CheckAction::Nudge(_)))
            .count()
    }

    fn inboxes(actions: &[CheckAction]) -> usize {
        actions
            .iter()
            .filter(|a| matches!(a, CheckAction::Inbox(..)))
            .count()
    }

    #[test]
    fn classify_blocked_beats_waiting() {
        let o = WorkerObservation {
            blocked: true,
            waiting: true,
            ..obs()
        };
        assert_eq!(classify(&o), WorkerState::BlockedPermission);
    }

    #[test]
    fn classify_trust_only_when_not_ready() {
        let not_ready = WorkerObservation {
            ready: false,
            pane: trust_pane(),
            ..obs()
        };
        assert_eq!(classify(&not_ready), WorkerState::BlockedTrust);
        let ready = WorkerObservation {
            ready: true,
            ..not_ready
        };
        assert_ne!(classify(&ready), WorkerState::BlockedTrust);
    }

    #[test]
    fn classify_trust_beats_waiting() {
        let o = WorkerObservation {
            ready: false,
            waiting: true,
            pane: trust_pane(),
            ..obs()
        };
        assert_eq!(classify(&o), WorkerState::BlockedTrust);
    }

    #[test]
    fn classify_waiting_beats_reported() {
        let o = WorkerObservation {
            reported_since_nudge: true,
            ..waiting()
        };
        assert_eq!(classify(&o), WorkerState::Waiting);
    }

    #[test]
    fn classify_reported_beats_unknown() {
        let o = WorkerObservation {
            reported_since_nudge: true,
            pane: Some("x".to_string()),
            pane_changed: false,
            ..obs()
        };
        assert_eq!(classify(&o), WorkerState::Reported);
    }

    #[test]
    fn classify_unknown_needs_pane_and_no_change() {
        let unchanged = WorkerObservation {
            pane: Some("x".to_string()),
            pane_changed: false,
            ..obs()
        };
        assert_eq!(classify(&unchanged), WorkerState::Unknown);
        let no_pane = WorkerObservation {
            pane: None,
            pane_changed: false,
            ..obs()
        };
        assert_eq!(classify(&no_pane), WorkerState::Working);
        let changed = WorkerObservation {
            pane_changed: true,
            ..unchanged
        };
        assert_eq!(classify(&changed), WorkerState::Working);
    }

    #[test]
    fn first_waiting_nudges_then_informs() {
        let (actions, memory) = plan(&CheckMemory::default(), &[waiting()]);
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0], CheckAction::Nudge("w1".to_string()));
        assert!(matches!(&actions[1], CheckAction::Inbox(id, _) if id == "w1"));
        assert!(memory.workers["w1"].nudged);
    }

    #[test]
    fn waiting_again_does_nothing() {
        let (_, memory) = plan(&CheckMemory::default(), &[waiting()]);
        let (actions, _) = plan(&memory, &[waiting()]);
        assert!(actions.is_empty());
    }

    #[test]
    fn waiting_working_waiting_informs_without_nudge() {
        let (_, m1) = plan(&CheckMemory::default(), &[waiting()]);
        let (_, m2) = plan(&m1, &[obs()]);
        let (actions, _) = plan(&m2, &[waiting()]);
        assert_eq!(nudges(&actions), 0);
        assert_eq!(inboxes(&actions), 1);
    }

    #[test]
    fn report_resets_nudge() {
        let (_, m1) = plan(&CheckMemory::default(), &[waiting()]);
        let reported = WorkerObservation {
            reported_since_nudge: true,
            ..obs()
        };
        let (_, m2) = plan(&m1, &[reported]);
        assert!(!m2.workers["w1"].nudged);
        let (actions, _) = plan(&m2, &[waiting()]);
        assert_eq!(nudges(&actions), 1);
    }

    #[test]
    fn blocked_and_unknown_never_nudge() {
        let blocked = WorkerObservation {
            blocked: true,
            ..obs()
        };
        let trust = WorkerObservation {
            ready: false,
            pane: trust_pane(),
            ..obs()
        };
        let unknown = WorkerObservation {
            pane: Some("x".to_string()),
            pane_changed: false,
            ..obs()
        };
        for o in [blocked, trust, unknown] {
            let (actions, _) = plan(&CheckMemory::default(), &[o]);
            assert_eq!(nudges(&actions), 0);
            assert_eq!(inboxes(&actions), 1);
        }
    }

    #[test]
    fn one_inbox_per_state_change() {
        let blocked = WorkerObservation {
            blocked: true,
            ..obs()
        };
        let (first, memory) = plan(&CheckMemory::default(), std::slice::from_ref(&blocked));
        let (second, _) = plan(&memory, &[blocked]);
        assert_eq!(inboxes(&first), 1);
        assert!(second.is_empty());
    }

    #[test]
    fn working_and_reported_emit_nothing() {
        let reported = WorkerObservation {
            reported_since_nudge: true,
            ..obs()
        };
        let (actions, _) = plan(&CheckMemory::default(), &[obs(), reported]);
        assert!(actions.is_empty());
    }

    #[test]
    fn inbox_texts_point_at_session() {
        let states = [
            waiting(),
            WorkerObservation {
                blocked: true,
                ..obs()
            },
            WorkerObservation {
                ready: false,
                pane: trust_pane(),
                ..obs()
            },
            WorkerObservation {
                pane: Some("x".to_string()),
                pane_changed: false,
                ..obs()
            },
        ];
        for o in states {
            let (actions, _) = plan(&CheckMemory::default(), &[o]);
            let text = actions
                .iter()
                .find_map(|a| match a {
                    CheckAction::Inbox(_, text) => Some(text.clone()),
                    CheckAction::Nudge(_) => None,
                })
                .unwrap();
            assert!(text.ends_with("tmux attach -t sess"));
        }
    }

    #[test]
    fn unobserved_workers_are_dropped() {
        let other = WorkerObservation {
            id: "w2".to_string(),
            ..obs()
        };
        let (_, memory) = plan(&CheckMemory::default(), &[obs(), other.clone()]);
        assert_eq!(memory.workers.len(), 2);
        let (_, memory) = plan(&memory, &[other]);
        assert_eq!(memory.workers.keys().collect::<Vec<_>>(), vec!["w2"]);
    }
}
