use ralph_core::chat_process::SessionKey;
use ralph_core::chat_socket::{decode_line, encode_line, MAX_FRAME_BYTES};
use ralph_core::chat_status::Status;
use ralph_core::chat_thread::{AutoMode, ThreadName};
use ralph_core::chat_thread_cmd::{OpenOpts, ThreadOp};
use ralph_core::program_status::{report_for, Phase, Report};
use ralph_core::turn_event::TurnEvent;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
        ]
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
