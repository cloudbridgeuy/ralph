use crate::chat::host::HostEvent;
use crate::chat::session::{Event, Reply};
use ralph_core::chat_send_it::SendItCommand;
use ralph_core::chat_socket::{decode_line, encode_line, MAX_FRAME_BYTES};
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
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    Send { text: String },
    SendIt { command: SendItCommand, width: u16 },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    Reply(Box<Reply>),
    Event(Event),
    Host(HostEvent),
    Turn(TurnEvent),
    Error { message: String },
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
    use crate::chat::send_it::Screen;
    use ralph_core::program_status::{Kind, Report, State};
    use ralph_core::turn_event::Wait;

    fn reply(shown: Option<Option<String>>) -> Reply {
        Reply {
            shown,
            ..Reply::noted("hi")
        }
    }

    fn server_messages() -> Vec<ServerMsg> {
        vec![
            ServerMsg::Reply(Box::new(reply(None))),
            ServerMsg::Reply(Box::new(reply(Some(None)))),
            ServerMsg::Reply(Box::new(reply(Some(Some("x".into()))))),
            ServerMsg::Event(Event::Notice("n".into())),
            ServerMsg::Event(Event::Blocked("b".into())),
            ServerMsg::Host(HostEvent::Fresh {
                fresh: Vec::new(),
                unread: 2,
            }),
            ServerMsg::Host(HostEvent::Repaint),
            ServerMsg::Host(HostEvent::Status(Report {
                state: State::Blocked(Kind::Question),
                msg: Some("q".into()),
            })),
            ServerMsg::Turn(TurnEvent::Blocks(Vec::new())),
            ServerMsg::Turn(TurnEvent::Busy(Wait::Memory)),
            ServerMsg::Turn(TurnEvent::Resume(Wait::Tool)),
            ServerMsg::Turn(TurnEvent::Idle),
            ServerMsg::Error {
                message: "m".into(),
            },
        ]
    }

    fn client_messages() -> Vec<ClientMsg> {
        vec![
            ClientMsg::Send { text: "t".into() },
            ClientMsg::SendIt {
                command: SendItCommand::Save {
                    name: "n".into(),
                    ticket: Some("GUZ-1".into()),
                },
                width: 80,
            },
            ClientMsg::SendIt {
                command: SendItCommand::Go,
                width: 120,
            },
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

    #[test]
    fn a_reply_keeps_the_difference_between_no_screen_and_a_cleared_one() {
        for shown in [None, Some(None), Some(Some("x".to_string()))] {
            let frame = encode_frame(1, &ServerMsg::Reply(Box::new(reply(shown.clone()))));
            let decoded = decode_frame::<ServerMsg>(&frame).unwrap();
            let ServerMsg::Reply(back) = decoded.body else {
                panic!("not a reply");
            };
            assert_eq!(back.shown, shown);
            assert_eq!(back.screen, Screen::text("hi"));
            assert!(back.started.is_none() && back.proposal.is_none());
        }
    }

    #[test]
    fn another_version_is_refused_even_with_an_unknown_body() {
        let line = r#"{"v":2,"id":1,"body":{"type":"from_the_future"}}"#;
        let error = decode_frame::<ClientMsg>(line).unwrap_err();
        assert!(error.contains('2'), "{error}");
    }

    #[test]
    fn a_frame_over_the_cap_is_refused() {
        let text = "a".repeat(MAX_FRAME_BYTES);
        let line = encode_frame(1, &ClientMsg::Send { text });
        assert!(line.len() > MAX_FRAME_BYTES);
        assert!(decode_frame::<ClientMsg>(&line).is_err());
    }

    #[test]
    fn a_frame_at_the_cap_passes() {
        let overhead = encode_frame(
            1,
            &ClientMsg::Send {
                text: String::new(),
            },
        )
        .len();
        let text = "a".repeat(MAX_FRAME_BYTES - overhead);
        let line = encode_frame(1, &ClientMsg::Send { text });
        assert_eq!(line.len(), MAX_FRAME_BYTES);
        assert!(decode_frame::<ClientMsg>(&line).is_ok());
    }
}
