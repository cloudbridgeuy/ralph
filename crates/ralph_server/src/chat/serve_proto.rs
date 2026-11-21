use ralph_core::chat_process::SessionKey;
use ralph_core::chat_socket::{decode_line, encode_line, MAX_FRAME_BYTES};
use ralph_core::chat_status::Status;
use ralph_core::chat_thread::{AutoMode, ThreadName};
use ralph_core::chat_thread_cmd::{OpenOpts, ThreadOp};
use ralph_core::program_status::{report_for, Phase, Report};
use ralph_core::turn_event::TurnEvent;
use ralph_core::worker::BlockedContent;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const VERSION: u32 = 1;

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
    },
    Blocked {
        thread: Option<String>,
        worker: String,
        content: Option<BlockedContent>,
    },
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
            r#"{"v":1,"id":5,"body":{"type":"status","report":{"state":{"Blocked":"Question"},"msg":"2 worker reports"},"unread":2,"blocked":["a","b"],"turns":3}}"#
        );
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
            r#"{"v":1,"id":4,"body":{"type":"cancel","thread":"side"}}"#
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
            r#"{"v":1,"id":3,"body":{"type":"turn","event":{"Busy":"Tool"}}}"#
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
            r#"{"v":1,"id":2,"body":{"type":"thread","op":"pull","name":"auth"}}"#
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
        let line = r#"{"v":2,"id":1,"body":{"type":"from_the_future"}}"#;
        let error = decode_frame::<ClientMsg>(line).unwrap_err();
        assert!(error.contains('2'), "{error}");
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
}
