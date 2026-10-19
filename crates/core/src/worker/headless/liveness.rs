use super::{HeadlessState, Liveness, Phase, Scope, WorkerEvent};
use crate::worker::ActivityMark;

pub fn liveness(
    state: &HeadlessState,
    now: u64,
    quiet_after: u64,
) -> (HeadlessState, Option<String>) {
    if let Liveness::Dead { scope, reason } = &state.liveness {
        if state.alerted {
            return (state.clone(), None);
        }
        let who = match scope {
            Scope::Worker => "worker agent",
        };
        let next = HeadlessState {
            alerted: true,
            ..state.clone()
        };
        return (next, Some(format!("{who} connection ended: {reason}")));
    }
    let quiet_for = now.saturating_sub(state.last_activity);
    if state.phase != Phase::Running || quiet_for < quiet_after {
        let next = HeadlessState {
            liveness: Liveness::Live,
            ..state.clone()
        };
        return (next, None);
    }
    let alert = (!state.alerted).then(|| format!("quiet {quiet_for} s; may be stuck on a prompt"));
    let next = HeadlessState {
        liveness: Liveness::Quiet {
            since: state.last_activity,
        },
        alerted: true,
        ..state.clone()
    };
    (next, alert)
}

pub fn marker_state(
    activity: Option<&str>,
    gone: Option<&str>,
    blocked: bool,
) -> (Phase, Vec<WorkerEvent>) {
    let mark = activity.and_then(ActivityMark::parse);
    let phase = match mark {
        _ if blocked => Phase::Blocked,
        Some(ActivityMark { running: true, .. }) => Phase::Running,
        Some(_) => Phase::Idle,
        None => Phase::Starting,
    };
    let events = mark
        .map(|mark| WorkerEvent::Activity { at: mark.at })
        .into_iter()
        .chain(gone.map(|reason| WorkerEvent::ProcessGone {
            scope: Scope::Worker,
            reason: reason.trim().to_string(),
        }))
        .collect();
    (phase, events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::reduce;

    fn state(phase: Phase, last_activity: u64) -> HeadlessState {
        HeadlessState {
            phase,
            last_activity,
            ..HeadlessState::default()
        }
    }

    fn feed(state: &HeadlessState, event: WorkerEvent) -> HeadlessState {
        reduce(state, &event).0
    }

    #[test]
    fn idle_blocked_and_starting_are_never_quiet() {
        for phase in [Phase::Idle, Phase::Blocked, Phase::Starting, Phase::Closed] {
            let (next, alert) = liveness(&state(phase, 10), 10_000, 300);
            assert_eq!((next.liveness, alert), (Liveness::Live, None));
        }
    }

    #[test]
    fn running_quiet_alerts_once() {
        let (quiet, alert) = liveness(&state(Phase::Running, 100), 400, 300);
        assert_eq!(quiet.liveness, Liveness::Quiet { since: 100 });
        assert_eq!(
            alert.as_deref(),
            Some("quiet 300 s; may be stuck on a prompt")
        );
        assert_eq!(liveness(&quiet, 500, 300).1, None);
    }

    #[test]
    fn activity_starts_a_new_quiet_episode() {
        let (quiet, _) = liveness(&state(Phase::Running, 100), 400, 300);
        let same = feed(&quiet, WorkerEvent::Activity { at: 100 });
        assert!(same.alerted);
        let fresh = feed(&quiet, WorkerEvent::Activity { at: 450 });
        assert_eq!(fresh.liveness, Liveness::Live);
        assert!(liveness(&fresh, 800, 300).1.is_some());
    }

    #[test]
    fn dead_alerts_once_and_ignores_activity() {
        let gone = WorkerEvent::ProcessGone {
            scope: Scope::Worker,
            reason: "agent acp exited".into(),
        };
        let dead = feed(&state(Phase::Running, 5), gone.clone());
        let (alerted, alert) = liveness(&dead, 6, 300);
        assert!(alert.is_some_and(|text| text.contains("agent acp exited")));
        let again = feed(&alerted, gone);
        assert_eq!(liveness(&again, 7, 300).1, None);
        let revived = feed(&again, WorkerEvent::Activity { at: 9 });
        assert_eq!(revived.last_activity, 5);
    }

    #[test]
    fn markers_map_to_phase_and_events() {
        assert_eq!(marker_state(None, None, false), (Phase::Starting, vec![]));
        assert_eq!(
            marker_state(Some("7 running"), None, true).0,
            Phase::Blocked
        );
        let (phase, events) = marker_state(Some("7 idle"), Some("boom\n"), false);
        assert_eq!(phase, Phase::Idle);
        assert_eq!(
            events,
            vec![
                WorkerEvent::Activity { at: 7 },
                WorkerEvent::ProcessGone {
                    scope: Scope::Worker,
                    reason: "boom".into()
                }
            ]
        );
    }
}
