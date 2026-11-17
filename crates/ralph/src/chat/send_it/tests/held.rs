use super::*;
use crate::chat::keepalive::Stamp;
use crate::chat::monitor::{Monitor, ProcessMemo};
use crate::chat::term_status::TermStatus;
use crate::schedule_store::ScheduleStore;
use ralph_core::chat::ResetLimits;
use ralph_core::chat_process::{FileStamp, UsageTail};
use ralph_core::chat_send_it::Awaiting;
use ralph_core::chat_status::Status;
use ralph_core::inbox_alert::{AlertConfig, UnknownFocus};
use ralph_server::chat::check::CheckState;
use std::collections::HashSet;
use std::sync::atomic::AtomicBool;

pub(in crate::chat) fn monitor(dir: &Path) -> Monitor {
    let inbox = Arc::new(Inbox::new(dir.join("inbox")));
    let store = Store::new(dir.to_path_buf());
    Monitor {
        watches: Watches::new(Arc::clone(&inbox), dir, std::time::Duration::from_secs(60)),
        schedule: ScheduleStore::new(dir.join("schedule")),
        inbox,
        store_dir: dir.to_path_buf(),
        status: Mutex::new(Status::default()),
        seen: Mutex::new(HashMap::new()),
        active: Mutex::new(None),
        recent_history_tokens: 0,
        worker_check_secs: None,
        worker_quiet_secs: 0,
        auto_continue: false,
        hold_progress: false,
        workers: Workers::new(
            dir.to_path_buf(),
            PathBuf::from("ralph"),
            String::new(),
            None,
        ),
        check: Mutex::new(CheckState::seeded(&store)),
        store,
        known: Mutex::new(Vec::new()),
        scheduled_reports: Mutex::new(Vec::new()),
        warned: Mutex::new(HashSet::new()),
        unrecorded: Mutex::new(HashSet::new()),
        alert: AlertConfig {
            sound: false,
            notification: false,
            unknown_focus: UnknownFocus::Silent,
            terminal_status: false,
        },
        live: Arc::new(AtomicBool::new(false)),
        last_ping: Stamp::default(),
        run: String::new(),
        reset_limits: ResetLimits {
            turns: 0,
            tokens: 0,
        },
        process: Mutex::new(ProcessMemo::default()),
        usage_tail: Mutex::new(UsageTail::starting_at(&FileStamp { id: None, len: 0 })),
        term: Arc::new(TermStatus::new(false, false)),
        phase: Mutex::new(Phase::Turn),
        hold: Mutex::new(None),
    }
}

fn reads_inbox(route: Option<Route>) -> bool {
    matches!(route, Some(Route::ReadInbox(Selection::At(position))) if position == Position::FIRST)
}

fn with_outcome(active: &Active, outcome: WatchOutcome) -> Active {
    Active {
        driving: active.driving.clone(),
        watched: Some(OutcomeSlot::new(Mutex::new(Some(outcome)))),
    }
}

#[test]
fn with_no_run_the_held_item_is_read_from_the_inbox() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, _) = started(dir.path());
    let mut sending = None;
    let route = send_it
        .held_item(&mut sending, Position::FIRST, &monitor(dir.path()))
        .unwrap()
        .route;
    assert!(reads_inbox(route));
    assert!(sending.is_none());
}

#[test]
fn a_run_not_awaiting_a_check_keeps_its_state_and_reads_the_inbox() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, active) = started(dir.path());
    let mut turn = with_outcome(&active, WatchOutcome::Exited(0));
    turn.driving.awaiting = Awaiting::Turn;
    let expected = turn.driving.clone();
    let mut sending = Some(turn);
    let route = send_it
        .held_item(&mut sending, Position::FIRST, &monitor(dir.path()))
        .unwrap()
        .route;
    assert!(reads_inbox(route));
    assert_eq!(sending.unwrap().driving, expected);
}

#[test]
fn a_check_with_no_outcome_yet_keeps_its_state_and_reads_the_inbox() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, active) = started(dir.path());
    let expected = active.driving.clone();
    let mut sending = Some(active);
    let route = send_it
        .held_item(&mut sending, Position::FIRST, &monitor(dir.path()))
        .unwrap()
        .route;
    assert!(reads_inbox(route));
    assert_eq!(sending.unwrap().driving, expected);
}

#[test]
fn a_settled_check_dispatches_with_no_inbox_read() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, active) = started(dir.path());
    let monitor = monitor(dir.path());
    let mut sending = Some(with_outcome(&active, WatchOutcome::Exited(0)));
    let route = send_it
        .held_item(&mut sending, Position::FIRST, &monitor)
        .unwrap()
        .route;
    assert!(!reads_inbox(route));
    assert!(sending.is_none());
}

#[test]
fn a_settle_that_cannot_drain_the_inbox_keeps_the_run() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, active) = started(dir.path());
    let monitor = monitor(dir.path());
    let inbox = dir.path().join("inbox");
    let _ = fs::remove_dir_all(&inbox);
    fs::write(&inbox, "not a directory").unwrap();
    let settled = with_outcome(&active, WatchOutcome::Exited(0));
    let expected = settled.driving.clone();
    let mut sending = Some(settled);
    let result = send_it.held_item(&mut sending, Position::FIRST, &monitor);
    assert!(result.is_err());
    assert_eq!(sending.unwrap().driving, expected);
}

#[test]
fn an_unread_message_is_briefed_once_and_marks_the_run_briefed() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, mut active) = started(dir.path());
    active.driving.briefed = false;
    let mut sending = Some(active);
    let first = send_it.brief_unread(&mut sending, "[w] done".to_string());
    assert!(first.starts_with("send-it run "), "{first}");
    assert!(first.ends_with("[w] done"), "{first}");
    assert!(sending.as_ref().unwrap().driving.briefed);
    let second = send_it.brief_unread(&mut sending, "[w] done".to_string());
    assert_eq!(second, "[w] done");
}

#[test]
fn an_unread_message_with_no_run_is_left_as_it_is() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, _) = started(dir.path());
    let mut sending = None;
    let message = send_it.brief_unread(&mut sending, "[w] done".to_string());
    assert_eq!(message, "[w] done");
    assert!(sending.is_none());
}

#[test]
fn a_settled_failing_check_that_stops_the_run_carries_the_go_prefill() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, active) = started(dir.path());
    let mut sending = Some(with_outcome(&active, WatchOutcome::Exited(1)));
    let driven = send_it
        .held_item(&mut sending, Position::FIRST, &monitor(dir.path()))
        .unwrap();
    assert_eq!(driven.prefill.as_deref(), Some("/send-it go"));
    assert!(driven.route.is_none());
}

#[test]
fn a_settled_passing_check_on_the_last_step_finishes_with_no_prefill() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, active) = started(dir.path());
    let mut sending = Some(with_outcome(&active, WatchOutcome::Exited(0)));
    let driven = send_it
        .held_item(&mut sending, Position::FIRST, &monitor(dir.path()))
        .unwrap();
    assert!(driven.prefill.is_none());
    assert!(driven.route.is_none());
    assert!(driven.active.is_none());
}

#[test]
#[ignore = "GUZ-1681 red until the fix"]
fn taking_over_a_run_carries_no_prefill() {
    let dir = tempfile::tempdir().unwrap();
    let (send_it, active) = started(dir.path());
    let driven = send_it.took_over(&active, &monitor(dir.path()));
    assert_eq!(driven.prefill, None);
    assert!(driven.route.is_none());
    assert!(driven.active.is_none());
}

#[test]
fn a_judge_turn_whose_plan_is_gone_carries_the_status_prefill() {
    let dir = tempfile::tempdir().unwrap();
    let send_it = SendIt::new(dir.path(), super::LIMITS);
    let mut args = super::plan_args();
    args["steps"] =
        serde_json::json!([{"do": "review", "criterion": {"kind": "judge", "question": "ok?"}}]);
    send_it.propose(&args).unwrap();
    let hash = send_it.status().shown.unwrap();
    let started = super::go(&send_it, Some(&hash), true).1.unwrap();
    let active = Active {
        driving: started.driving,
        watched: None,
    };
    let ticked = send_it.tick(&active, super::ended());
    assert_eq!(ticked.next, Next::Judge);
    fs::remove_file(&send_it.path).unwrap();
    let driven = send_it.dispatch(ticked, &monitor(dir.path()));
    assert_eq!(driven.prefill.as_deref(), Some("/send-it status"));
    assert!(driven.route.is_none());
}

#[test]
fn a_check_that_cannot_start_carries_the_go_prefill() {
    let dir = tempfile::tempdir().unwrap();
    let send_it = SendIt::new(dir.path(), super::LIMITS);
    let mut args = super::plan_args();
    args["steps"] = serde_json::json!([{"do": "wait", "criterion": {"kind": "check_passes", "command": "sleep 30"}}]);
    send_it.propose(&args).unwrap();
    let hash = send_it.status().shown.unwrap();
    let ticked = super::go(&send_it, Some(&hash), true).1.unwrap();
    let Next::Check(command) = ticked.next.clone() else {
        panic!("expected a check, got {:?}", ticked.next);
    };
    let monitor = monitor(dir.path());
    monitor
        .watches
        .start(check_spec(ticked.driving.progress.step, &command))
        .unwrap();
    let driven = send_it.dispatch(ticked, &monitor);
    assert_eq!(driven.prefill.as_deref(), Some("/send-it go"));
    assert!(driven.route.is_none());
    assert!(driven.active.is_none());
}

#[test]
#[ignore = "GUZ-1681 red until the fix"]
fn a_typed_draft_is_kept_over_the_driven_prefill() {
    let mut editor = Reedline::create();
    editor.run_edit_commands(&prefill_commands(Some("my draft")));
    let driven = Driven {
        prefill: Some("/send-it go".to_string()),
        ..Driven::ended()
    };
    assert!(driven.enter(&mut editor).is_none());
    assert_eq!(editor.current_buffer_contents(), "my draft");
}

#[test]
fn an_empty_buffer_takes_the_driven_prefill() {
    let mut editor = Reedline::create();
    let driven = Driven {
        prefill: Some("/send-it go".to_string()),
        ..Driven::ended()
    };
    assert!(driven.enter(&mut editor).is_none());
    assert_eq!(editor.current_buffer_contents(), "/send-it go");
}

#[test]
fn a_prefill_becomes_one_insert_command() {
    assert_eq!(
        prefill_commands(Some("/send-it go")),
        vec![EditCommand::InsertString("/send-it go".to_string())]
    );
}

#[test]
fn no_prefill_makes_no_command() {
    assert_eq!(prefill_commands(None), Vec::new());
}
