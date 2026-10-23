use super::{HeadlessState, Liveness, Phase, Scope, WorkerEvent};
use crate::worker::{parse_gone, ActivityMark};
use std::collections::{BTreeMap, BTreeSet, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlertKind {
    Quiet,
    Dead { scope: Scope, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    pub kind: AlertKind,
    pub text: String,
}

pub fn group_alerts(alerts: Vec<(String, Alert)>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut servers: BTreeMap<String, (String, Vec<String>)> = BTreeMap::new();
    for (worker, alert) in alerts {
        match alert.kind {
            AlertKind::Dead {
                scope: Scope::Server(name),
                reason,
            } => servers
                .entry(name)
                .or_insert_with(|| (reason, Vec::new()))
                .1
                .push(worker),
            _ => out.push((worker, alert.text)),
        }
    }
    for (name, (reason, mut workers)) in servers {
        workers.sort();
        let joined = workers.join(", ");
        let text = format!(
            "{}; affected workers: {joined}",
            dead_text(&Scope::Server(name), &reason)
        );
        out.push((joined, text));
    }
    out
}

fn dead_text(scope: &Scope, reason: &str) -> String {
    match scope {
        Scope::Worker => format!("worker agent connection ended: {reason}"),
        Scope::Server(name) => format!("opencode server {name} ended: {reason}"),
    }
}

pub fn liveness(
    state: &HeadlessState,
    now: u64,
    quiet_after: u64,
) -> (HeadlessState, Option<Alert>) {
    if let Liveness::Dead { scope, reason } = &state.liveness {
        if state.alerted {
            return (state.clone(), None);
        }
        let next = HeadlessState {
            alerted: true,
            ..state.clone()
        };
        let alert = Alert {
            kind: AlertKind::Dead {
                scope: scope.clone(),
                reason: reason.clone(),
            },
            text: dead_text(scope, reason),
        };
        return (next, Some(alert));
    }
    let quiet_for = now.saturating_sub(state.last_activity);
    if state.phase != Phase::Running || quiet_for < quiet_after {
        let next = HeadlessState {
            liveness: Liveness::Live,
            ..state.clone()
        };
        return (next, None);
    }
    let alert = (!state.alerted).then(|| Alert {
        kind: AlertKind::Quiet,
        text: format!("quiet {quiet_for} s; may be stuck on a prompt"),
    });
    let next = HeadlessState {
        liveness: Liveness::Quiet {
            since: state.last_activity,
        },
        alerted: true,
        ..state.clone()
    };
    (next, alert)
}

pub fn gone_acknowledged(state: &HeadlessState) -> bool {
    state.alerted
        && matches!(
            state.liveness,
            Liveness::Dead {
                scope: Scope::Server(_),
                ..
            }
        )
}

pub fn vanished_claude(
    was_alive: &HashSet<String>,
    alive_now: &HashSet<String>,
    ready: &[String],
) -> Vec<String> {
    ready
        .iter()
        .filter(|id| was_alive.contains(*id) && !alive_now.contains(*id))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub fn rearm(state: HeadlessState, gone: bool) -> HeadlessState {
    if !gone && matches!(state.liveness, Liveness::Dead { .. }) {
        return HeadlessState::default();
    }
    state
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
        .chain(gone.map(|text| {
            let (scope, reason) = parse_gone(text);
            WorkerEvent::ProcessGone { scope, reason }
        }))
        .collect();
    (phase, events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::{reduce, BlockedContent};

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
            alert.map(|alert| alert.text).as_deref(),
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
        assert!(alert.is_some_and(|alert| alert.text.contains("agent acp exited")));
        let again = feed(&alerted, gone);
        assert_eq!(liveness(&again, 7, 300).1, None);
        let revived = feed(&again, WorkerEvent::Activity { at: 9 });
        assert_eq!(revived.last_activity, 5);
    }

    fn dead_server_state(alerted: bool) -> HeadlessState {
        HeadlessState {
            liveness: Liveness::Dead {
                scope: Scope::Server("pid 7".into()),
                reason: "exited".into(),
            },
            alerted,
            ..HeadlessState::default()
        }
    }

    #[test]
    fn only_alerted_server_death_is_acknowledged() {
        assert!(gone_acknowledged(&dead_server_state(true)));
        assert!(!gone_acknowledged(&dead_server_state(false)));
        let worker_dead = feed(
            &state(Phase::Running, 5),
            WorkerEvent::ProcessGone {
                scope: Scope::Worker,
                reason: "x".into(),
            },
        );
        let (worker_dead, _) = liveness(&worker_dead, 6, 300);
        assert!(!gone_acknowledged(&worker_dead));
        let (quiet, _) = liveness(&state(Phase::Running, 100), 400, 300);
        assert!(!gone_acknowledged(&quiet));
        assert!(!gone_acknowledged(&state(Phase::Idle, 1)));
    }

    #[test]
    fn rearm_resets_only_dead_state_without_a_marker() {
        let dead = dead_server_state(true);
        assert_eq!(rearm(dead.clone(), false), HeadlessState::default());
        assert_eq!(rearm(dead.clone(), true), dead);
        let live = state(Phase::Running, 5);
        assert_eq!(rearm(live.clone(), false), live);
        let (quiet, _) = liveness(&state(Phase::Running, 100), 400, 300);
        assert_eq!(rearm(quiet.clone(), false), quiet);
    }

    #[test]
    fn repeated_server_death_alerts_each_time_after_acknowledgement() {
        let text = "scope server pid 7\ngone";
        let tick = |previous: HeadlessState, gone: Option<&str>| {
            let (phase, events) = marker_state(Some("7 idle"), gone, false);
            let mut current = HeadlessState {
                phase,
                ..rearm(previous, gone.is_some())
            };
            for event in &events {
                current = feed(&current, event.clone());
            }
            liveness(&current, 8, 300)
        };
        let (first, alert) = tick(HeadlessState::default(), Some(text));
        assert!(alert.is_some());
        assert!(gone_acknowledged(&first));
        let (still, alert) = tick(first, Some(text));
        assert_eq!(alert, None);
        assert!(gone_acknowledged(&still));
        let (cleared, alert) = tick(still, None);
        assert_eq!(alert, None);
        assert!(!gone_acknowledged(&cleared));
        let (second, alert) = tick(cleared, Some(text));
        assert!(alert.is_some());
        assert!(gone_acknowledged(&second));
    }

    fn server_dead(name: &str) -> Alert {
        let scope = Scope::Server(name.into());
        Alert {
            text: dead_text(&scope, "exited"),
            kind: AlertKind::Dead {
                scope,
                reason: "exited".into(),
            },
        }
    }

    fn quiet() -> Alert {
        Alert {
            kind: AlertKind::Quiet,
            text: "quiet".into(),
        }
    }

    #[test]
    fn one_server_death_yields_one_alert_naming_every_worker() {
        let alerts = ["C", "A", "B"]
            .map(|id| (id.to_string(), server_dead("pid 7")))
            .to_vec();
        let grouped = group_alerts(alerts);
        assert_eq!(grouped.len(), 1);
        assert_eq!(grouped[0].0, "A, B, C");
        assert!(grouped[0].1.contains("pid 7"));
        assert!(grouped[0].1.contains("exited"));
        assert!(grouped[0].1.ends_with("A, B, C"));
    }

    #[test]
    fn quiet_and_other_servers_stay_separate() {
        let grouped = group_alerts(vec![
            ("A".into(), quiet()),
            ("B".into(), quiet()),
            ("C".into(), server_dead("pid 1")),
            ("D".into(), server_dead("pid 2")),
        ]);
        assert_eq!(grouped.len(), 4);
        assert_eq!(grouped[0], ("A".to_string(), "quiet".to_string()));
    }

    #[test]
    fn markers_map_to_phase_and_events() {
        assert_eq!(marker_state(None, None, false), (Phase::Starting, vec![]));
        assert_eq!(
            marker_state(Some("7 running"), None, true).0,
            Phase::Blocked
        );
        let (phase, events) = marker_state(Some("7 idle"), Some("boom\n"), false);
        let server = marker_state(None, Some("scope server pid 7\ngone"), false).1;
        assert_eq!(
            server,
            vec![WorkerEvent::ProcessGone {
                scope: Scope::Server("pid 7".into()),
                reason: "gone".into()
            }]
        );
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

    #[test]
    fn codex_episode_alerts_once_per_quiet_then_dead() {
        let mut current = feed(&HeadlessState::default(), WorkerEvent::Activity { at: 100 });
        current = feed(&current, WorkerEvent::TurnStarted);
        assert_eq!(current.phase, Phase::Running);
        let (quiet, alert) = liveness(&current, 400, 300);
        assert!(alert.is_some());
        assert_eq!(liveness(&quiet, 500, 300).1, None);
        let live = feed(&quiet, WorkerEvent::Activity { at: 600 });
        assert_eq!(live.liveness, Liveness::Live);
        let (quiet, alert) = liveness(&live, 900, 300);
        assert!(alert.is_some());
        let gone = WorkerEvent::ProcessGone {
            scope: Scope::Worker,
            reason: "codex app-server exited".into(),
        };
        let (dead, alert) = liveness(&feed(&quiet, gone), 901, 300);
        assert!(alert.is_some_and(|alert| alert.text.contains("codex app-server exited")));
        let after = feed(&dead, WorkerEvent::Activity { at: 950 });
        assert_eq!(liveness(&after, 2000, 300).1, None);
    }

    #[test]
    fn blocked_turn_is_never_quiet() {
        let running = feed(&HeadlessState::default(), WorkerEvent::TurnStarted);
        let blocked = feed(
            &feed(&running, WorkerEvent::Activity { at: 10 }),
            WorkerEvent::PermissionAsked(BlockedContent {
                tool: "bash".into(),
                summary: "ls".into(),
                options: Vec::new(),
            }),
        );
        assert_eq!(blocked.phase, Phase::Blocked);
        let (after, alert) = liveness(&blocked, 10_000, 300);
        assert_eq!((after.liveness, alert), (Liveness::Live, None));
        let (phase, events) = marker_state(Some("10 running"), None, true);
        assert_eq!(phase, Phase::Blocked);
        let from_marker = events
            .iter()
            .fold(state(phase, 0), |acc, event| feed(&acc, event.clone()));
        assert_eq!(liveness(&from_marker, 10_000, 300).1, None);
    }

    fn ids(items: &[&str]) -> HashSet<String> {
        items.iter().map(|id| id.to_string()).collect()
    }

    #[test]
    fn vanished_claude_returns_sorted_ready_sessions_that_disappeared() {
        let ready = vec!["C".to_string(), "A".to_string()];
        assert_eq!(
            vanished_claude(&ids(&["A", "C"]), &ids(&[]), &ready),
            vec!["A".to_string(), "C".to_string()]
        );
    }

    #[test]
    fn vanished_claude_ignores_clean_close_alive_and_new_ids() {
        let ready = vec!["A".to_string(), "B".to_string(), "N".to_string()];
        assert!(vanished_claude(&ids(&["X"]), &ids(&[]), &ready).is_empty());
        assert!(vanished_claude(&ids(&["A"]), &ids(&["A"]), &ready).is_empty());
        assert!(vanished_claude(&ids(&[]), &ids(&["N"]), &ready).is_empty());
        assert!(vanished_claude(&ids(&["B"]), &ids(&[]), &[]).is_empty());
    }
}
