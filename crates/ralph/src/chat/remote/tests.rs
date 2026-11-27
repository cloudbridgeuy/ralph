use super::*;
use ralph_core::inbox_alert::UnknownFocus;
use ralph_core::program_status::{Report, State};
use ralph_core::stream::blocks::{
    OutputBlock, ToolInvocationBlock, ToolInvocationVariant, ToolResultBlock, ToolResultVariant,
};
use ralph_core::worker::{BlockedContent, PermissionOption, Report as WorkerReport, ReportStatus};

fn event() -> TurnEvent {
    TurnEvent::Warning("hi".into())
}

#[test]
fn a_turn_event_is_rendered() {
    let step = on_frame(ServerMsg::Turn { event: event() });
    assert_eq!(step, Step::Render(event()));
}

fn kept_block() -> OutputBlock {
    OutputBlock::Separator
}

fn bash_blocks() -> Vec<OutputBlock> {
    vec![
        OutputBlock::ToolInvocation(ToolInvocationBlock {
            tool_name: "Bash".into(),
            variant: ToolInvocationVariant::Bash {
                command: "ls".into(),
                description: None,
            },
        }),
        OutputBlock::ToolResult(ToolResultBlock {
            tool_name: "Bash".into(),
            is_error: false,
            variant: ToolResultVariant::Bash {
                content: Some("out".into()),
                truncated: false,
            },
        }),
    ]
}

#[test]
fn without_a_terminal_tool_blocks_are_dropped_and_other_blocks_stay() {
    let mut blocks = vec![kept_block()];
    blocks.extend(bash_blocks());
    assert_eq!(remote_blocks(blocks, false), vec![kept_block()]);
}

#[test]
fn on_a_terminal_every_block_stays() {
    let mut blocks = vec![kept_block()];
    blocks.extend(bash_blocks());
    assert_eq!(remote_blocks(blocks.clone(), true), blocks);
}

#[test]
fn turn_done_finishes_the_turn() {
    assert_eq!(on_frame(ServerMsg::TurnDone), Step::Finished);
}

#[test]
fn inbox_push_displays_the_session_report_as_a_notice() {
    for (thread, label) in [(None, "main"), (Some("side"), "side")] {
        let frame = serde_json::json!({"v": VERSION, "id": 0, "body": {
            "type": "inbox", "thread": thread, "unread": 3,
            "report": {"worker": "w1", "status": "done", "at": "now", "text": "tests\npassed"}
        }});
        let step = decode_frame::<ServerMsg>(&frame.to_string()).map(|frame| on_frame(frame.body));
        assert_eq!(
            step,
            Ok(plain(&format!("{label}/w1 done (3 unread): tests passed")))
        );
    }
}

fn plain(line: &str) -> Step {
    Step::Push(Push {
        line: line.into(),
        needs_user: None,
    })
}

fn inbox_step(status: &str) -> Result<Step, String> {
    let frame = serde_json::json!({"v": VERSION, "id": 0, "body": {
        "type": "inbox", "thread": null, "unread": 1,
        "report": {"worker": "w1", "status": status, "at": "1970-01-01T00:00:00Z", "text": "hi"}
    }});
    decode_frame::<ServerMsg>(&frame.to_string()).map(|frame| on_frame(frame.body))
}

#[test]
fn an_inbox_push_that_needs_the_user_carries_the_report_to_alert_on() {
    for status in [ReportStatus::Blocked, ReportStatus::Question] {
        let name = status.as_str();
        assert_eq!(
            inbox_step(name),
            Ok(Step::Push(Push {
                line: format!("main/w1 {name} (1 unread): hi"),
                needs_user: Some(WorkerReport::from_worker(
                    &ralph_core::worker::WorkerId::parse("w1").unwrap(),
                    chrono::DateTime::UNIX_EPOCH,
                    status,
                    "hi".into()
                )),
            }))
        );
    }
}

#[test]
fn an_inbox_push_that_informs_or_has_an_unknown_status_is_a_plain_notice() {
    for status in ["done", "check", "report", "failed", "from-the-future"] {
        assert_eq!(
            inbox_step(status),
            Ok(plain(&format!("main/w1 {status} (1 unread): hi"))),
            "{status}"
        );
    }
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
        stage: 0,
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
        plain("side/w1 blocked: Bash ls request r1 options a|b")
    );
    let settled = on_frame(ServerMsg::Blocked {
        thread: None,
        worker: "w1".into(),
        content: None,
    });
    assert_eq!(settled, plain("w1 permission settled"));
}

#[test]
fn wait_step_cancels_once_and_exits_when_terminated() {
    assert_eq!(wait_step(false, true, false), WaitStep::SendCancel);
    assert_eq!(wait_step(false, true, true), WaitStep::Continue);
    assert_eq!(wait_step(false, false, false), WaitStep::Continue);
    assert_eq!(wait_step(false, false, true), WaitStep::Continue);
    assert_eq!(wait_step(true, false, false), WaitStep::Exit);
    assert_eq!(wait_step(true, true, false), WaitStep::Exit);
    assert_eq!(wait_step(true, true, true), WaitStep::Exit);
}

#[test]
fn prompt_step_sends_lines_and_ignores_other_signals() {
    assert_eq!(
        prompt_step(Signal::Success("hi".into()), false),
        PromptStep::Send("hi".into())
    );
    assert_eq!(prompt_step(Signal::CtrlC, false), PromptStep::Continue);
    assert_eq!(
        prompt_step(Signal::ExternalBreak(String::new()), false),
        PromptStep::Continue
    );
}

#[test]
fn ctrl_d_on_connect_is_a_user_quit() {
    assert_eq!(
        prompt_step(Signal::CtrlD, false),
        PromptStep::Exit(Departure::UserQuit)
    );
}

#[test]
fn a_signal_on_connect_is_not_a_user_quit() {
    for signal in [
        Signal::CtrlC,
        Signal::CtrlD,
        Signal::ExternalBreak("draft".into()),
        Signal::Success("hi".into()),
    ] {
        assert_eq!(
            prompt_step(signal, true),
            PromptStep::Exit(Departure::Restart)
        );
    }
}

#[test]
fn read_prompt_until_stops_on_interrupted_when_terminated() {
    let mut calls = 0;
    let flag = std::cell::Cell::new(false);
    let stopped = read_prompt_until(
        || {
            calls += 1;
            if calls == 1 {
                flag.set(true);
                Err(io::Error::from(io::ErrorKind::Interrupted))
            } else {
                Ok("unreachable")
            }
        },
        || flag.get(),
    )
    .unwrap();
    assert_eq!(stopped, None);
    assert_eq!(calls, 1);

    let mut retries = 0;
    let resumed = read_prompt_until(
        || {
            retries += 1;
            if retries < 3 {
                Err(io::Error::from(io::ErrorKind::Interrupted))
            } else {
                Ok("ok")
            }
        },
        || false,
    )
    .unwrap();
    assert_eq!(resumed, Some("ok"));
    assert_eq!(retries, 3);

    let mut skipped_calls = 0;
    let skipped = read_prompt_until(
        || {
            skipped_calls += 1;
            Ok("unreachable")
        },
        || true,
    )
    .unwrap();
    assert_eq!(skipped, None);
    assert_eq!(skipped_calls, 0);
}

struct NoopSink;

impl TurnSink for NoopSink {
    fn emit(&self, _event: TurnEvent) {}
}

fn pair_session(timeout: Duration) -> (RemoteSession, UnixStream) {
    let (client, server) = UnixStream::pair().unwrap();
    client.set_read_timeout(Some(timeout)).unwrap();
    let session = RemoteSession {
        socket: PathBuf::from("/nonexistent/serve.sock"),
        stream: RefCell::new(BufReader::new(client)),
        next_id: Cell::new(1),
        turns: Cell::new(0),
        attached: RefCell::new(None),
        sink: Box::new(NoopSink),
        stdout_is_tty: false,
        alert: AlertConfig {
            sound: false,
            notification: false,
            unknown_focus: UnknownFocus::Silent,
            terminal_status: None,
        },
    };
    (session, server)
}

#[test]
fn read_frame_with_returns_none_when_terminated_without_reading() {
    let (session, mut server) = pair_session(Duration::from_millis(20));
    server
        .write_all(encode_frame(1, &ServerMsg::TurnDone).as_bytes())
        .unwrap();
    let cancel = CancelToken::default();
    cancel.cancel();
    let mut sent = false;
    let frame = session
        .read_frame_with(&mut String::new(), Some((&cancel, &mut sent)), || true)
        .unwrap();
    assert!(frame.is_none());
    assert!(!sent);
}

#[test]
fn read_frame_with_exits_after_poll_timeout_when_terminated_flips() {
    let (session, _server) = pair_session(Duration::from_millis(5));
    let checks = Cell::new(0u8);
    let cancel = CancelToken::default();
    let mut sent = false;
    let frame = session
        .read_frame_with(&mut String::new(), Some((&cancel, &mut sent)), || {
            let n = checks.get() + 1;
            checks.set(n);
            n >= 2
        })
        .unwrap();
    assert!(frame.is_none());
    assert!(!sent);
    assert_eq!(checks.get(), 2);
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
        status_summary(None, State::Idle, 2, &blocked, 0),
        "main: idle, 2 unread, blocked: a, b"
    );
    assert_eq!(
        status_summary(Some("fix"), State::Working, 0, &[], 3),
        "fix: working, 0 unread, stage 3"
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
