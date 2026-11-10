use super::*;

const IDLE_FOOTER: &str = "─────\n❯\n─────\n  ? for shortcuts";
const BUSY_FOOTER: &str = "─────\n❯\n─────\n  esc to interrupt";

fn busy_marked(state: ActivityState, age: u64, pane: Option<&str>) -> WorkerObservation {
    WorkerObservation {
        id: "w1".to_string(),
        session: "sess".to_string(),
        kind: HarnessId::ClaudeTmux,
        ready: true,
        blocked: false,
        waiting: false,
        unreported: false,
        reported_since_nudge: false,
        pane: pane.map(str::to_string),
        activity: Some(state),
        activity_age_secs: age,
        auto_continue: true,
    }
}

#[test]
fn a_running_mark_goes_stale_after_ten_minutes() {
    assert!(!busy_mark_stale(ActivityState::Running, 600));
    assert!(busy_mark_stale(ActivityState::Running, 601));
}

#[test]
fn a_background_mark_goes_stale_after_an_hour() {
    assert!(!busy_mark_stale(ActivityState::Background, 601));
    assert!(!busy_mark_stale(ActivityState::Background, 3600));
    assert!(busy_mark_stale(ActivityState::Background, 3601));
}

#[test]
fn idle_and_failed_marks_are_never_busy_stale() {
    for state in [ActivityState::Idle, ActivityState::Failed] {
        assert!(!busy_mark_stale(state, u64::MAX));
    }
}

#[test]
fn a_stale_running_mark_with_an_idle_pane_is_unknown() {
    let observation = busy_marked(ActivityState::Running, 601, Some(IDLE_FOOTER));
    assert_eq!(classify(&observation), WorkerState::Unknown);
}

#[test]
fn a_stale_background_mark_with_an_idle_pane_is_unknown() {
    let observation = busy_marked(ActivityState::Background, 3601, Some(IDLE_FOOTER));
    assert_eq!(classify(&observation), WorkerState::Unknown);
}

#[test]
fn fresh_busy_marks_stay_working_with_an_idle_pane() {
    for (state, age) in [
        (ActivityState::Running, 600),
        (ActivityState::Background, 3600),
    ] {
        let observation = busy_marked(state, age, Some(IDLE_FOOTER));
        assert_eq!(classify(&observation), WorkerState::Working, "{state:?}");
    }
}

#[test]
fn a_stale_running_mark_stays_working_while_the_pane_is_busy() {
    let observation = busy_marked(ActivityState::Running, 5000, Some(BUSY_FOOTER));
    assert_eq!(classify(&observation), WorkerState::Working);
}

#[test]
fn a_stale_mark_without_a_pane_stays_working() {
    let observation = busy_marked(ActivityState::Running, 5000, None);
    assert_eq!(classify(&observation), WorkerState::Working);
}

#[test]
fn a_stale_mark_alerts_unknown_once() {
    let observation = busy_marked(ActivityState::Running, 601, Some(IDLE_FOOTER));
    let (first, memory) = plan(&CheckMemory::default(), std::slice::from_ref(&observation));
    let (second, _) = plan(&memory, &[observation]);
    let inbox = |actions: &[CheckAction]| {
        actions
            .iter()
            .filter(|action| matches!(action, CheckAction::Inbox(..)))
            .count()
    };
    assert_eq!(inbox(&first), 1);
    assert_eq!(inbox(&second), 0);
}
