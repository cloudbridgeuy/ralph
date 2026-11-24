use ralph_core::chat_process::SessionKey;
use ralph_core::chat_socket::{decode_line, encode_line, MAX_FRAME_BYTES};
use ralph_core::chat_status::Status;
use ralph_core::chat_thread::{AutoMode, ThreadName};
use ralph_core::chat_thread_cmd::{OpenOpts, ThreadOp};
use ralph_core::program_status::{report_for, Phase, Report};
use ralph_core::stream::blocks::{OutputBlock, ToolResultVariant};
use ralph_core::turn_event::TurnEvent;
use ralph_core::worker::{BlockedContent, Report as InboxReport};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope<B> {
    pub v: u32,
    pub id: u64,
    pub body: B,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ThreadCmd {
    Pull {
        name: String,
    },
    Park {
        name: String,
    },
    Revive {
        name: String,
    },
    Drop {
        name: String,
    },
    Merge {
        name: String,
        view: bool,
    },
    Open {
        name: String,
        model: Option<String>,
        auto: Option<AutoMode>,
    },
    Switch {
        name: String,
    },
    Main,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadRequest {
    Op(ThreadOp),
    Attach(SessionKey),
}

impl ThreadCmd {
    pub fn parse(self) -> Result<ThreadRequest, String> {
        let name = |name: &str| ThreadName::parse(name);
        let op = |op: ThreadOp| ThreadRequest::Op(op);
        match self {
            ThreadCmd::Pull { name: n } => name(&n).map(|n| op(ThreadOp::Pull(n))),
            ThreadCmd::Park { name: n } => name(&n).map(|n| op(ThreadOp::Park(n))),
            ThreadCmd::Revive { name: n } => name(&n).map(|n| op(ThreadOp::Revive(n))),
            ThreadCmd::Drop { name: n } => name(&n).map(|n| op(ThreadOp::Drop(n))),
            ThreadCmd::Merge { name: n, view } => {
                name(&n).map(|name| op(ThreadOp::Merge { name, view }))
            }
            ThreadCmd::Open {
                name: n,
                model,
                auto,
            } => name(&n).map(|name| op(ThreadOp::Open(OpenOpts { name, auto, model }))),
            ThreadCmd::Switch { name: n } => {
                name(&n).map(|n| ThreadRequest::Attach(SessionKey::Thread(n)))
            }
            ThreadCmd::Main => Ok(ThreadRequest::Attach(SessionKey::Main)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    Hello {
        proto: u32,
    },
    Send {
        thread: Option<String>,
        text: String,
    },
    Cancel {
        thread: Option<String>,
    },
    Status {
        thread: Option<String>,
    },
    Thread(ThreadCmd),
    Answer {
        thread: Option<String>,
        worker: String,
        request: String,
        option: String,
    },
    Shutdown,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    Error {
        message: String,
    },
    Hello {
        proto: u32,
    },
    Turn {
        event: TurnEvent,
    },
    TurnDone,
    Done {
        message: String,
    },
    Status {
        report: Report,
        unread: usize,
        blocked: Vec<String>,
        turns: u64,
        #[serde(default)]
        stage: usize,
    },
    Blocked {
        thread: Option<String>,
        worker: String,
        content: Option<BlockedContent>,
    },
    Inbox {
        thread: Option<String>,
        report: InboxReport,
        unread: usize,
    },
}

pub fn inbox_frame(key: &SessionKey, report: InboxReport, unread: usize) -> ServerMsg {
    let bytes = report.worker.len() + report.status.len() + report.at.len() + report.text.len();
    let thread = match key {
        SessionKey::Main => None,
        SessionKey::Thread(name) => Some(name.to_string()),
    };
    let mut frame = ServerMsg::Inbox {
        thread,
        report,
        unread,
    };
    if bytes <= MAX_FRAME_BYTES / 8
        || (bytes < MAX_FRAME_BYTES && encode_frame(0, &frame).len() <= MAX_FRAME_BYTES)
    {
        return frame;
    }
    if let ServerMsg::Inbox { report, .. } = &mut frame {
        const MAX_FIELD_BYTES: usize = MAX_FRAME_BYTES / 32;
        for field in [
            &mut report.worker,
            &mut report.at,
            &mut report.status,
            &mut report.text,
        ] {
            if field.len() > MAX_FIELD_BYTES {
                field.truncate(field.floor_char_boundary(MAX_FIELD_BYTES - '…'.len_utf8()));
                field.push('…');
            }
        }
    }
    frame
}

pub fn turn_frame(event: TurnEvent) -> ServerMsg {
    let mut frame = ServerMsg::Turn { event };
    if encode_frame(0, &frame).len() <= MAX_FRAME_BYTES {
        return frame;
    }
    let mut cap = turn_blocks(&mut frame).map_or(0, |blocks| largest_result_text(blocks));
    while cap > 0 && encode_frame(0, &frame).len() > MAX_FRAME_BYTES {
        cap /= 2;
        if let Some(blocks) = turn_blocks(&mut frame) {
            blocks.iter_mut().for_each(|block| cut_result(block, cap));
        }
    }
    frame
}

fn turn_blocks(frame: &mut ServerMsg) -> Option<&mut Vec<OutputBlock>> {
    match frame {
        ServerMsg::Turn {
            event: TurnEvent::Blocks(blocks),
        } => Some(blocks),
        _ => None,
    }
}

fn largest_result_text(blocks: &mut [OutputBlock]) -> usize {
    blocks
        .iter_mut()
        .filter_map(result_variant)
        .flat_map(|variant| result_text(variant).0)
        .map(|text| text.len())
        .max()
        .unwrap_or(0)
}

fn result_variant(block: &mut OutputBlock) -> Option<&mut ToolResultVariant> {
    match block {
        OutputBlock::ToolResult(result) => Some(&mut result.variant),
        _ => None,
    }
}

fn cut_result(block: &mut OutputBlock, cap: usize) {
    let Some(variant) = result_variant(block) else {
        return;
    };
    let (texts, flag) = result_text(variant);
    let mut cut = false;
    for text in texts {
        if text.len() > cap {
            text.truncate(text.floor_char_boundary(cap.saturating_sub('…'.len_utf8())));
            text.push('…');
            cut = true;
        }
    }
    if let (true, Some(flag)) = (cut, flag) {
        *flag = true;
    }
}

fn result_text(variant: &mut ToolResultVariant) -> (Vec<&mut String>, Option<&mut bool>) {
    match variant {
        ToolResultVariant::Bash { content, truncated } => {
            (content.iter_mut().collect(), Some(truncated))
        }
        ToolResultVariant::EditBeforeAfter {
            old_content,
            new_content,
            ..
        } => (vec![old_content, new_content], None),
        ToolResultVariant::EditDiff { diff_content, .. } => (vec![diff_content], None),
        ToolResultVariant::WriteNewFile { content, .. }
        | ToolResultVariant::Grep { content, .. } => (vec![content], None),
        ToolResultVariant::WriteOverwrite {
            before_content,
            after_content,
            ..
        } => (vec![before_content, after_content], None),
        ToolResultVariant::Read {
            content, truncated, ..
        }
        | ToolResultVariant::Glob {
            content, truncated, ..
        } => (vec![content], Some(truncated)),
        ToolResultVariant::TodoWrite { message } => (message.iter_mut().collect(), None),
        ToolResultVariant::NotebookEdit { diff_content, .. } => {
            (diff_content.iter_mut().collect(), None)
        }
        ToolResultVariant::ToolError { content } | ToolResultVariant::Default { content } => {
            (content.iter_mut().collect(), None)
        }
        ToolResultVariant::Script {
            logs,
            result,
            error,
            ..
        } => (
            logs.iter_mut()
                .chain(result.iter_mut())
                .chain(error.iter_mut())
                .collect(),
            None,
        ),
        ToolResultVariant::EditNoChanges { .. } | ToolResultVariant::WriteNoChanges { .. } => {
            (Vec::new(), None)
        }
    }
}

pub type BlockedKey = (Option<String>, String);
pub type BlockedMap = BTreeMap<BlockedKey, BlockedContent>;

pub fn blocked_pushes(prev: &BlockedMap, now: &BlockedMap) -> Vec<ServerMsg> {
    let push =
        |(thread, worker): &BlockedKey, content: Option<&BlockedContent>| ServerMsg::Blocked {
            thread: thread.clone(),
            worker: worker.clone(),
            content: content.cloned(),
        };
    let raised = now
        .iter()
        .filter(|(key, content)| prev.get(*key) != Some(content))
        .map(|(key, content)| push(key, Some(content)));
    let settled = prev
        .keys()
        .filter(|key| !now.contains_key(*key))
        .map(|key| push(key, None));
    raised.chain(settled).collect()
}

pub fn status_frame(status: &Status, turns: u64) -> ServerMsg {
    let mut blocked: Vec<String> = status
        .workers
        .iter()
        .filter(|worker| worker.is_blocked())
        .map(|worker| worker.id.clone())
        .collect();
    blocked.sort();
    ServerMsg::Status {
        report: report_for(Phase::Prompt, status),
        unread: status.unread,
        blocked,
        turns,
        stage: status.stage,
    }
}

pub fn hello_reply(proto: u32) -> ServerMsg {
    if proto == VERSION {
        ServerMsg::Hello { proto: VERSION }
    } else {
        ServerMsg::Error {
            message: format!("Unsupported serve protocol {proto}, expected {VERSION}"),
        }
    }
}

pub fn encode_frame<B: Serialize>(id: u64, body: &B) -> String {
    encode_line(&Envelope {
        v: VERSION,
        id,
        body,
    })
}

pub fn decode_frame<B: DeserializeOwned>(line: &str) -> Result<Envelope<B>, String> {
    if line.len() > MAX_FRAME_BYTES {
        return Err(format!("Frame exceeds {MAX_FRAME_BYTES} bytes"));
    }
    let Envelope { v, id, body } = decode_line::<Envelope<Value>>(line)?;
    if v != VERSION {
        return Err(format!(
            "Unsupported protocol version {v}, expected {VERSION}"
        ));
    }
    let body = serde_json::from_value(body).map_err(|e| format!("Malformed frame body: {e}"))?;
    Ok(Envelope { v, id, body })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::chat_status::WorkerState;
    use ralph_core::program_status::{Kind, State};
    use ralph_core::turn_event::Wait;

    fn bash_result(content: String) -> OutputBlock {
        OutputBlock::tool_result(
            "Bash",
            false,
            ToolResultVariant::Bash {
                content: Some(content),
                truncated: false,
            },
        )
    }

    fn bash_truncated(frame: &ServerMsg) -> Vec<bool> {
        let ServerMsg::Turn {
            event: TurnEvent::Blocks(blocks),
        } = frame
        else {
            return Vec::new();
        };
        blocks
            .iter()
            .filter_map(|block| match block {
                OutputBlock::ToolResult(result) => match &result.variant {
                    ToolResultVariant::Bash { truncated, .. } => Some(*truncated),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_blocks_event_over_the_limit_is_cut_to_fit_and_flags_each_cut_result() {
        let each = "x".repeat(MAX_FRAME_BYTES * 3 / 5);
        let blocks = vec![bash_result(each.clone()), bash_result(each)];
        let frame = turn_frame(TurnEvent::Blocks(blocks));
        assert!(encode_frame(0, &frame).len() <= MAX_FRAME_BYTES);
        assert_eq!(bash_truncated(&frame), [true, true]);
    }

    #[test]
    fn a_blocks_event_under_the_limit_is_unchanged() {
        let event = TurnEvent::Blocks(vec![bash_result("é".repeat(1000))]);
        let plain = ServerMsg::Turn {
            event: event.clone(),
        };
        assert_eq!(encode_frame(0, &turn_frame(event)), encode_frame(0, &plain));
    }

    fn server_messages() -> Vec<ServerMsg> {
        vec![
            ServerMsg::Error {
                message: "m".into(),
            },
            ServerMsg::Hello { proto: VERSION },
            ServerMsg::Turn {
                event: TurnEvent::Busy(Wait::Tool),
            },
            ServerMsg::Turn {
                event: TurnEvent::Idle,
            },
            ServerMsg::TurnDone,
            ServerMsg::Done {
                message: "ok".into(),
            },
            ServerMsg::Status {
                report: Report {
                    state: State::Blocked(Kind::Question),
                    msg: Some("1 worker report".into()),
                },
                unread: 1,
                blocked: vec!["w1".into()],
                turns: 4,
                stage: 2,
            },
            ServerMsg::Blocked {
                thread: Some("side".into()),
                worker: "w1".into(),
                content: Some(content("r1", "Bash")),
            },
            ServerMsg::Blocked {
                thread: None,
                worker: "w1".into(),
                content: None,
            },
        ]
    }

    fn content(request_id: &str, tool: &str) -> BlockedContent {
        BlockedContent {
            request_id: request_id.into(),
            tool: tool.into(),
            summary: "ls".into(),
            options: Vec::new(),
            pending: Vec::new(),
        }
    }

    fn key(thread: Option<&str>, worker: &str) -> BlockedKey {
        (thread.map(str::to_string), worker.to_string())
    }

    fn pushes(prev: &BlockedMap, now: &BlockedMap) -> Vec<serde_json::Value> {
        blocked_pushes(prev, now)
            .iter()
            .map(|msg| serde_json::to_value(msg).unwrap())
            .collect()
    }

    fn blocked(thread: Option<&str>, worker: &str, content: Option<BlockedContent>) -> Value {
        serde_json::to_value(ServerMsg::Blocked {
            thread: thread.map(str::to_string),
            worker: worker.into(),
            content,
        })
        .unwrap()
    }

    #[test]
    fn blocked_pushes_report_new_changed_and_settled_requests() {
        let first = content("r1", "Bash");
        let second = content("r2", "Bash");
        let none = BlockedMap::new();
        let one = BlockedMap::from([(key(None, "w1"), first.clone())]);
        assert_eq!(
            pushes(&none, &one),
            vec![blocked(None, "w1", Some(first.clone()))]
        );
        assert!(pushes(&one, &one).is_empty());
        let changed = BlockedMap::from([(key(None, "w1"), second.clone())]);
        assert_eq!(
            pushes(&one, &changed),
            vec![blocked(None, "w1", Some(second))]
        );
        let side = BlockedMap::from([(key(Some("side"), "w1"), first)]);
        assert_eq!(
            pushes(&side, &none),
            vec![blocked(Some("side"), "w1", None)]
        );
    }

    fn client_messages() -> Vec<ClientMsg> {
        vec![
            ClientMsg::Hello { proto: VERSION },
            ClientMsg::Send {
                thread: None,
                text: "hi".into(),
            },
            ClientMsg::Send {
                thread: Some("side".into()),
                text: "hi".into(),
            },
            ClientMsg::Thread(ThreadCmd::Merge {
                name: "side".into(),
                view: true,
            }),
            ClientMsg::Cancel { thread: None },
            ClientMsg::Cancel {
                thread: Some("side".into()),
            },
            ClientMsg::Status { thread: None },
            ClientMsg::Shutdown,
        ]
    }

    #[test]
    fn client_messages_round_trip() {
        for msg in client_messages() {
            let decoded = decode_frame::<ClientMsg>(&encode_frame(7, &msg)).unwrap();
            assert_eq!(
                decoded,
                Envelope {
                    v: VERSION,
                    id: 7,
                    body: msg
                }
            );
        }
    }

    #[test]
    fn server_messages_round_trip() {
        for msg in server_messages() {
            let decoded = decode_frame::<ServerMsg>(&encode_frame(9, &msg)).unwrap();
            assert_eq!((decoded.v, decoded.id), (VERSION, 9));
            assert_eq!(
                serde_json::to_value(&decoded.body).unwrap(),
                serde_json::to_value(&msg).unwrap()
            );
        }
    }

    fn worker(id: &str, state: &str) -> WorkerState {
        WorkerState {
            id: id.into(),
            state: state.into(),
        }
    }

    #[test]
    fn status_frame_reports_sorted_blocked_workers_and_unread() {
        let status = Status {
            unread: 2,
            workers: vec![
                worker("b", "permission"),
                worker("c", "running"),
                worker("a", "permission"),
            ],
            ..Status::default()
        };
        let line = encode_frame(5, &status_frame(&status, 3));
        assert_eq!(
            line.trim_end(),
            r#"{"v":2,"id":5,"body":{"type":"status","report":{"state":{"Blocked":"Question"},"msg":"2 worker reports"},"unread":2,"blocked":["a","b"],"turns":3,"stage":0}}"#
        );
    }

    #[test]
    fn the_stage_count_survives_a_status_round_trip() {
        let status = Status {
            stage: 3,
            ..Status::default()
        };
        let line = encode_frame(1, &status_frame(&status, 0));
        let decoded = decode_frame::<ServerMsg>(&line).unwrap();
        assert!(matches!(decoded.body, ServerMsg::Status { stage: 3, .. }));
    }

    #[test]
    fn a_cancel_travels_under_the_cancel_type() {
        let line = encode_frame(
            4,
            &ClientMsg::Cancel {
                thread: Some("side".into()),
            },
        );
        assert_eq!(
            line.trim_end(),
            r#"{"v":2,"id":4,"body":{"type":"cancel","thread":"side"}}"#
        );
    }

    #[test]
    fn a_turn_event_travels_under_the_turn_type() {
        let line = encode_frame(
            3,
            &ServerMsg::Turn {
                event: TurnEvent::Busy(Wait::Tool),
            },
        );
        assert_eq!(
            line.trim_end(),
            r#"{"v":2,"id":3,"body":{"type":"turn","event":{"Busy":"Tool"}}}"#
        );
    }

    #[test]
    fn a_thread_command_travels_under_the_thread_type() {
        let line = encode_frame(
            2,
            &ClientMsg::Thread(ThreadCmd::Pull {
                name: "auth".into(),
            }),
        );
        assert_eq!(
            line.trim_end(),
            r#"{"v":2,"id":2,"body":{"type":"thread","op":"pull","name":"auth"}}"#
        );
    }

    #[test]
    fn a_thread_command_parses_names_at_the_boundary() {
        let merge = ThreadCmd::Merge {
            name: "auth".into(),
            view: true,
        };
        assert_eq!(
            merge.parse(),
            Ok(ThreadRequest::Op(ThreadOp::Merge {
                name: ThreadName::parse("auth").unwrap(),
                view: true
            }))
        );
        assert!(ThreadCmd::Drop {
            name: "../x".into()
        }
        .parse()
        .is_err());
        assert!(ThreadCmd::Pull {
            name: "main".into()
        }
        .parse()
        .is_err());
    }

    #[test]
    fn open_and_switch_parse_to_requests() {
        let line = r#"{"type":"thread","op":"open","name":"auth","auto":"merge"}"#;
        let ClientMsg::Thread(open) = serde_json::from_str(line).unwrap() else {
            panic!("expected a thread command");
        };
        let auth = ThreadName::parse("auth").unwrap();
        assert_eq!(
            open.parse(),
            Ok(ThreadRequest::Op(ThreadOp::Open(OpenOpts {
                name: auth.clone(),
                auto: Some(AutoMode::Merge),
                model: None
            })))
        );
        assert_eq!(
            ThreadCmd::Switch {
                name: "auth".into()
            }
            .parse(),
            Ok(ThreadRequest::Attach(SessionKey::Thread(auth)))
        );
        assert_eq!(
            ThreadCmd::Main.parse(),
            Ok(ThreadRequest::Attach(SessionKey::Main))
        );
    }

    #[test]
    fn hello_is_answered_with_the_served_version() {
        assert!(matches!(
            hello_reply(VERSION),
            ServerMsg::Hello { proto } if proto == VERSION
        ));
    }

    #[test]
    fn hello_with_another_version_is_an_error_naming_both() {
        let ServerMsg::Error { message } = hello_reply(VERSION + 1) else {
            panic!("not an error");
        };
        assert!(message.contains(&(VERSION + 1).to_string()), "{message}");
        assert!(message.contains("expected"), "{message}");
    }

    #[test]
    fn another_version_is_refused_even_with_an_unknown_body() {
        let line = r#"{"v":3,"id":1,"body":{"type":"from_the_future"}}"#;
        let error = decode_frame::<ClientMsg>(line).unwrap_err();
        assert!(error.contains('3'), "{error}");
    }

    #[test]
    fn a_frame_over_the_cap_is_refused() {
        let message = "a".repeat(MAX_FRAME_BYTES);
        let line = encode_frame(1, &ServerMsg::Error { message });
        assert!(line.len() > MAX_FRAME_BYTES);
        assert!(decode_frame::<ServerMsg>(&line).is_err());
    }

    #[test]
    fn a_frame_at_the_cap_passes() {
        let overhead = encode_frame(
            1,
            &ServerMsg::Error {
                message: String::new(),
            },
        )
        .len();
        let message = "a".repeat(MAX_FRAME_BYTES - overhead);
        let line = encode_frame(1, &ServerMsg::Error { message });
        assert_eq!(line.len(), MAX_FRAME_BYTES);
        assert!(decode_frame::<ServerMsg>(&line).is_ok());
    }

    #[test]
    fn inbox_push_preserves_large_reports_that_fit_in_one_frame() {
        let report = InboxReport {
            worker: "w".into(),
            at: "now".into(),
            status: "done".into(),
            text: "a".repeat(MAX_FRAME_BYTES / 2),
        };
        let frame = inbox_frame(&SessionKey::Main, report.clone(), 1);
        assert!(matches!(frame, ServerMsg::Inbox { report: received, .. } if received == report));
    }

    #[test]
    fn inbox_push_stays_within_the_frame_limit_for_escaped_and_multibyte_text() {
        let field = "\u{0000}".repeat(MAX_FRAME_BYTES / 5);
        let report = InboxReport {
            worker: field.clone(),
            at: field.clone(),
            status: field,
            text: "🦀".repeat(MAX_FRAME_BYTES / 4),
        };
        let line = encode_frame(0, &inbox_frame(&SessionKey::Main, report, 1));
        assert!(line.len() <= MAX_FRAME_BYTES);
        let body = decode_frame::<ServerMsg>(&line).unwrap().body;
        assert!(matches!(body, ServerMsg::Inbox { report, .. } if report.text.ends_with('…')));
    }
}
