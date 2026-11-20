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
    Hello {
        proto: u32,
    },
    Send {
        thread: Option<String>,
        text: String,
    },
    Shutdown,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    Error { message: String },
    Hello { proto: u32 },
    Turn { event: TurnEvent },
    TurnDone,
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
