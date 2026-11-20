use super::{step, Host, POISONED};
use ralph_core::chat_socket::MAX_FRAME_BYTES;
use ralph_core::serve_supervisor::{Effect, Input};
use ralph_server::chat::serve_proto::{
    decode_frame, encode_frame, hello_reply, ClientMsg, ServerMsg,
};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::Ordering;
use std::time::Duration;

const READ_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, PartialEq, Eq)]
enum Action {
    Hello(u32),
    Shutdown,
    Refuse(String),
}

pub(super) fn handle(stream: UnixStream, host: &Host) -> std::io::Result<()> {
    stream.set_read_timeout(read_timeout(false))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    let mut greeted = false;
    while let Some(line) = read_frame(&mut reader)? {
        let frame = decode_frame::<ClientMsg>(line.trim_end());
        let id = frame.as_ref().map_or(0, |envelope| envelope.id);
        let reply = match classify(greeted, frame.map(|envelope| envelope.body)) {
            Action::Shutdown => {
                host.stop.store(true, Ordering::SeqCst);
                return Ok(());
            }
            Action::Refuse(message) => ServerMsg::Error { message },
            Action::Hello(proto) => match step(&host.supervisor, Input::Hello { proto }) {
                Ok(effects) => answer(&effects),
                Err(message) => ServerMsg::Error { message },
            },
        };
        writer.write_all(encode_frame(id, &reply).as_bytes())?;
        if matches!(reply, ServerMsg::Error { .. }) {
            return Ok(());
        }
        greeted = true;
        writer.set_read_timeout(read_timeout(greeted))?;
    }
    Ok(())
}

fn read_timeout(greeted: bool) -> Option<Duration> {
    (!greeted).then_some(READ_TIMEOUT)
}

fn read_frame(reader: &mut impl BufRead) -> std::io::Result<Option<String>> {
    let mut line = String::new();
    let read = reader
        .by_ref()
        .take(MAX_FRAME_BYTES as u64 + 1)
        .read_line(&mut line)?;
    Ok((read > 0).then_some(line))
}

fn classify(greeted: bool, frame: Result<ClientMsg, String>) -> Action {
    match frame {
        Err(message) => Action::Refuse(message),
        Ok(ClientMsg::Hello { proto }) => Action::Hello(proto),
        Ok(_) if !greeted => Action::Refuse("unexpected message before hello".to_string()),
        Ok(ClientMsg::Shutdown) => Action::Shutdown,
    }
}

fn answer(effects: &[Effect]) -> ServerMsg {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Answer { proto } => Some(hello_reply(*proto)),
            Effect::Refuse(message) => Some(ServerMsg::Error {
                message: message.clone(),
            }),
            _ => None,
        })
        .unwrap_or_else(|| ServerMsg::Error {
            message: POISONED.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answer_maps_hello_and_refusals() {
        assert!(matches!(
            answer(&[Effect::Answer { proto: 1 }]),
            ServerMsg::Hello { proto: 1 }
        ));
        assert!(matches!(
            answer(&[Effect::Answer { proto: 2 }]),
            ServerMsg::Error { .. }
        ));
        let ServerMsg::Error { message } = answer(&[Effect::Refuse("down".into())]) else {
            panic!("expected error");
        };
        assert_eq!(message, "down");
    }

    #[test]
    fn a_greeted_connection_has_no_read_timeout() {
        assert_eq!(read_timeout(false), Some(READ_TIMEOUT));
        assert_eq!(read_timeout(true), None);
    }

    #[test]
    fn classify_gates_on_hello() {
        assert_eq!(
            classify(false, Ok(ClientMsg::Hello { proto: 1 })),
            Action::Hello(1)
        );
        assert_eq!(
            classify(false, Ok(ClientMsg::Shutdown)),
            Action::Refuse("unexpected message before hello".into())
        );
        assert_eq!(classify(true, Ok(ClientMsg::Shutdown)), Action::Shutdown);
        assert_eq!(
            classify(true, Err("bad".into())),
            Action::Refuse("bad".into())
        );
    }
}
