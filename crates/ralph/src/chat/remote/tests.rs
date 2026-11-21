use super::*;
use ralph_core::program_status::{Report, State};

fn event() -> TurnEvent {
    TurnEvent::Warning("hi".into())
}

#[test]
fn a_turn_event_is_rendered() {
    let step = on_frame(ServerMsg::Turn { event: event() });
    assert_eq!(step, Step::Render(event()));
}

#[test]
fn turn_done_finishes_the_turn() {
    assert_eq!(on_frame(ServerMsg::TurnDone), Step::Finished);
}

#[test]
fn an_error_frame_fails_the_turn() {
    let step = on_frame(ServerMsg::Error {
        message: "busy".into(),
    });
    assert_eq!(step, Step::Failed("busy".into()));
}

#[test]
fn frames_that_do_not_belong_mid_turn_are_ignored() {
    let status = ServerMsg::Status {
        report: Report {
            state: State::Idle,
            msg: None,
        },
        unread: 0,
        blocked: Vec::new(),
        turns: 0,
    };
    for msg in [
        ServerMsg::Hello { proto: 1 },
        ServerMsg::Done {
            message: "ok".into(),
        },
        status,
    ] {
        assert_eq!(on_frame(msg), Step::Ignored);
    }
}

#[test]
fn cancel_is_due_once_per_turn() {
    assert!(cancel_due(true, false));
    assert!(!cancel_due(true, true));
    assert!(!cancel_due(false, false));
    assert!(!cancel_due(false, true));
}
