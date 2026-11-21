use super::*;
use ralph_core::program_status::{Report, State};
use ralph_core::worker::PermissionOption;

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
fn a_blocked_push_becomes_a_notice() {
    let content = BlockedContent {
        request_id: "r1".into(),
        tool: "Bash".into(),
        summary: "ls".into(),
        options: vec![
            PermissionOption {
                option_id: "a".into(),
                name: "Allow".into(),
                kind: "allow_once".into(),
            },
            PermissionOption {
                option_id: "b".into(),
                name: "Deny".into(),
                kind: "reject_once".into(),
            },
        ],
        pending: Vec::new(),
    };
    let raised = on_frame(ServerMsg::Blocked {
        thread: Some("side".into()),
        worker: "w1".into(),
        content: Some(content),
    });
    assert_eq!(
        raised,
        Step::Notice("side/w1 blocked: Bash ls request r1 options a|b".into())
    );
    let settled = on_frame(ServerMsg::Blocked {
        thread: None,
        worker: "w1".into(),
        content: None,
    });
    assert_eq!(settled, Step::Notice("w1 permission settled".into()));
}

#[test]
fn cancel_is_due_once_per_turn() {
    assert!(cancel_due(true, false));
    assert!(!cancel_due(true, true));
    assert!(!cancel_due(false, false));
    assert!(!cancel_due(false, true));
}

fn route(line: &str) -> Route {
    route_input(line, 0, &EmptySubmit::default(), &BTreeSet::new())
}

fn request(line: &str) -> Option<Result<ThreadCmd, String>> {
    thread_request(&route(line))
}

#[test]
fn status_summary_lists_unread_and_blocked_workers() {
    let blocked = vec!["a".to_string(), "b".to_string()];
    assert_eq!(
        status_summary(None, State::Idle, 2, &blocked),
        "main: idle, 2 unread, blocked: a, b"
    );
    assert_eq!(
        status_summary(Some("fix"), State::Working, 0, &[]),
        "fix: working, 0 unread"
    );
}

#[test]
fn thread_lines_map_to_frames() {
    let name = |n: &str| n.to_string();
    let cases = [
        ("/thread main", ThreadCmd::Main),
        (
            "/thread switch fix",
            ThreadCmd::Switch { name: name("fix") },
        ),
        ("/thread pull fix", ThreadCmd::Pull { name: name("fix") }),
        ("/thread park fix", ThreadCmd::Park { name: name("fix") }),
        (
            "/thread revive fix",
            ThreadCmd::Revive { name: name("fix") },
        ),
        ("/thread drop fix", ThreadCmd::Drop { name: name("fix") }),
        (
            "/thread merge fix --view",
            ThreadCmd::Merge {
                name: name("fix"),
                view: true,
            },
        ),
        (
            "/thread open fix --model opus",
            ThreadCmd::Open {
                name: name("fix"),
                model: Some(name("opus")),
                auto: None,
            },
        ),
    ];
    for (line, cmd) in cases {
        assert_eq!(request(line), Some(Ok(cmd)), "{line}");
    }
}

#[test]
fn ls_and_bad_thread_lines_are_refused_and_other_lines_are_not_threads() {
    assert!(matches!(request("/thread ls"), Some(Err(_))));
    assert!(matches!(request("/thread switch"), Some(Err(_))));
    assert_eq!(request("hello"), None);
}

#[test]
fn switch_and_main_move_the_attached_thread() {
    let switch = ThreadCmd::Switch { name: "fix".into() };
    assert_eq!(attached_after(None, &switch), Some("fix".into()));
    assert_eq!(attached_after(Some("fix".into()), &ThreadCmd::Main), None);
    let pull = ThreadCmd::Pull { name: "x".into() };
    assert_eq!(
        attached_after(Some("fix".into()), &pull),
        Some("fix".into())
    );
}

#[test]
fn only_server_refusals_keep_the_chat_open() {
    assert!(keeps_chat_open(&Refused("main is busy".into())));
    let closed: Box<dyn Error> = "ralph serve closed the connection".into();
    assert!(!keeps_chat_open(&*closed));
}
