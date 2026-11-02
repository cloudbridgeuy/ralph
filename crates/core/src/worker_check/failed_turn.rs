use super::*;

fn failed_idle(auto_continue: bool) -> WorkerObservation {
    WorkerObservation {
        id: "w1".to_string(),
        session: "sess".to_string(),
        kind: HarnessId::Claude,
        ready: true,
        blocked: false,
        waiting: false,
        unreported: false,
        reported_since_nudge: false,
        pane: Some("idle".to_string()),
        activity: Some(ActivityState::Failed),
        activity_age_secs: 0,
        auto_continue,
    }
}

fn inbox_count(actions: &[CheckAction]) -> usize {
    actions
        .iter()
        .filter(|action| matches!(action, CheckAction::Inbox(..)))
        .count()
}

#[test]
fn a_failed_turn_is_left_to_the_stall_module_when_auto_continue_is_on() {
    let observation = failed_idle(true);
    assert_eq!(classify(&observation), WorkerState::Working);
    let (actions, _) = plan(&CheckMemory::default(), &[observation]);
    assert_eq!(inbox_count(&actions), 0);
}

#[test]
fn a_failed_turn_still_alerts_unknown_when_auto_continue_is_off() {
    let observation = failed_idle(false);
    assert_eq!(classify(&observation), WorkerState::Unknown);
    let (actions, _) = plan(&CheckMemory::default(), &[observation]);
    assert_eq!(inbox_count(&actions), 1);
}

#[test]
fn an_idle_activity_still_alerts_unknown_when_auto_continue_is_on() {
    let observation = WorkerObservation {
        activity: Some(ActivityState::Idle),
        ..failed_idle(true)
    };
    assert_eq!(classify(&observation), WorkerState::Unknown);
}
