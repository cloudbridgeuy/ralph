use super::{drive, step, Host, POISONED};
use crate::chat::thread::open::prepare_open;
use crate::chat::thread::running_ids;
use crate::chat::thread_cmd::{merge_line, record_cli_merge, run as run_thread, ThreadEnv};
use ralph_core::chat_process::SessionKey;
use ralph_core::chat_socket::MAX_FRAME_BYTES;
use ralph_core::chat_thread::config_root;
use ralph_core::chat_thread::ThreadName;
use ralph_core::chat_thread_cmd::{OpenOpts, ThreadOp};
use ralph_core::serve_supervisor::{Effect, Input};
use ralph_server::chat::serve_proto::{
    decode_frame, encode_frame, hello_reply, ClientMsg, ServerMsg, ThreadRequest,
};
use std::cell::RefCell;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::Ordering;
use std::time::Duration;

const READ_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, PartialEq, Eq)]
enum Action {
    Hello(u32),
    Send { key: SessionKey, text: String },
    Thread(ThreadOp),
    Attach(SessionKey),
    Shutdown,
    Refuse(String),
}

pub(super) fn handle(stream: UnixStream, host: &Host) -> std::io::Result<()> {
    stream.set_read_timeout(read_timeout(false))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    let mut greeted = false;
    let mut attached = SessionKey::Main;
    while let Some(line) = read_frame(&mut reader)? {
        let frame = decode_frame::<ClientMsg>(line.trim_end());
        let id = frame.as_ref().map_or(0, |envelope| envelope.id);
        let action = classify(greeted, &attached, frame.map(|envelope| envelope.body));
        let closes = closes_on_error(&action);
        let reply = match action {
            Action::Shutdown => {
                host.stop.store(true, Ordering::SeqCst);
                return Ok(());
            }
            Action::Refuse(message) => ServerMsg::Error { message },
            Action::Hello(proto) => match step(&host.supervisor, Input::Hello { proto }) {
                Ok(effects) => answer(&effects),
                Err(message) => ServerMsg::Error { message },
            },
            Action::Send { key, text } => send(host, &key, &text, id, &writer),
            Action::Thread(op) => thread(host, op),
            Action::Attach(key) => {
                attached = key;
                ServerMsg::Done {
                    message: format!("attached to {attached}"),
                }
            }
        };
        writer.write_all(encode_frame(id, &reply).as_bytes())?;
        if closes && matches!(reply, ServerMsg::Error { .. }) {
            return Ok(());
        }
        greeted = true;
        writer.set_read_timeout(read_timeout(greeted))?;
    }
    Ok(())
}

fn thread(host: &Host, op: ThreadOp) -> ServerMsg {
    let result = match op {
        ThreadOp::Open(opts) => open(host, &opts),
        op => step(&host.supervisor, Input::Thread(op))
            .and_then(|effects| {
                let op = permit_thread(&effects)?;
                drive(host, effects).map_err(|e| e.to_string())?;
                Ok(op)
            })
            .and_then(|op| run_thread_op(host, &op)),
    };
    match result {
        Ok(message) => ServerMsg::Done { message },
        Err(message) => ServerMsg::Error { message },
    }
}

fn open(host: &Host, opts: &OpenOpts) -> Result<String, String> {
    let key = SessionKey::Thread(opts.name.clone());
    let effects = step(&host.supervisor, Input::Open(key.clone()))?;
    permit_open(&effects)?;
    let opened = prepare_open(&host.store_dir, opts)
        .and_then(|_| drive(host, effects).map_err(|e| e.to_string()));
    match opened {
        Ok(_) => Ok(format!("{key} is open")),
        Err(message) => {
            step(&host.supervisor, Input::Closed(key))?;
            Err(message)
        }
    }
}

fn run_thread_op(host: &Host, op: &ThreadOp) -> Result<String, String> {
    let main = config_root(&host.store_dir);
    let notices = RefCell::new(Vec::new());
    let env = ThreadEnv {
        store_dir: &host.store_dir,
        in_thread: false,
        width: 80,
        display: None,
        notice: &|text| notices.borrow_mut().push(text.to_string()),
        curate: &|prompt, curator| crate::run_curator(prompt, curator),
    };
    let message = run_thread(op, &env, &mut |_| Some(true), &|name| {
        running_ids(&main, name)
    })?;
    if let ThreadOp::Merge { name, view } = op {
        record_cli_merge(&main, name, &merge_line(name, *view), &message);
    }
    let mut lines = notices.into_inner();
    lines.push(message);
    Ok(lines.join("\n"))
}

fn send(host: &Host, key: &SessionKey, text: &str, id: u64, writer: &UnixStream) -> ServerMsg {
    let run = step(&host.supervisor, Input::Send(key.clone())).and_then(|effects| permit(&effects));
    let result = run.and_then(|key| {
        let outcome = run_turn(host, &key, text, id, writer);
        step(&host.supervisor, Input::TurnDone(key))?;
        outcome
    });
    match result {
        Ok(()) => ServerMsg::TurnDone,
        Err(message) => ServerMsg::Error { message },
    }
}

fn run_turn(
    host: &Host,
    key: &SessionKey,
    text: &str,
    id: u64,
    writer: &UnixStream,
) -> Result<(), String> {
    let open = host
        .sessions
        .get(key)
        .ok_or_else(|| format!("{key} is not open"))?;
    let bound = writer.try_clone().map_err(|e| e.to_string())?;
    open.sink.bind(id, bound);
    let outcome = open.turn(text);
    open.sink.unbind();
    outcome.map(|_| ()).map_err(|e| e.to_string())
}

fn closes_on_error(action: &Action) -> bool {
    !matches!(
        action,
        Action::Send { .. } | Action::Thread(_) | Action::Attach(_)
    )
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

fn session_key(thread: Option<&str>, attached: &SessionKey) -> Result<SessionKey, String> {
    thread.map_or(Ok(attached.clone()), |name| {
        ThreadName::parse(name).map(SessionKey::Thread)
    })
}

fn classify(greeted: bool, attached: &SessionKey, frame: Result<ClientMsg, String>) -> Action {
    match frame {
        Err(message) => Action::Refuse(message),
        Ok(ClientMsg::Hello { proto }) => Action::Hello(proto),
        Ok(_) if !greeted => Action::Refuse("unexpected message before hello".to_string()),
        Ok(ClientMsg::Send { thread, text }) => match session_key(thread.as_deref(), attached) {
            Ok(key) => Action::Send { key, text },
            Err(message) => Action::Refuse(message),
        },
        Ok(ClientMsg::Thread(cmd)) => match cmd.parse() {
            Ok(ThreadRequest::Op(op)) => Action::Thread(op),
            Ok(ThreadRequest::Attach(key)) => Action::Attach(key),
            Err(message) => Action::Refuse(message),
        },
        Ok(ClientMsg::Shutdown) => Action::Shutdown,
    }
}

fn permit_open(effects: &[Effect]) -> Result<(), String> {
    match effects.iter().find_map(|effect| match effect {
        Effect::Refuse(message) => Some(message.clone()),
        _ => None,
    }) {
        Some(message) => Err(message),
        None => Ok(()),
    }
}

fn permit_thread(effects: &[Effect]) -> Result<ThreadOp, String> {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::RunThread(op) => Some(Ok(op.clone())),
            Effect::Refuse(message) => Some(Err(message.clone())),
            _ => None,
        })
        .unwrap_or_else(|| Err(POISONED.to_string()))
}

fn permit(effects: &[Effect]) -> Result<SessionKey, String> {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Run(key) => Some(Ok(key.clone())),
            Effect::Refuse(message) => Some(Err(message.clone())),
            _ => None,
        })
        .unwrap_or_else(|| Err(POISONED.to_string()))
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
    use ralph_server::chat::serve_proto::ThreadCmd;

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
    fn classify_routes_a_send_to_a_session_key() {
        let send = |thread: Option<&str>| {
            classify(
                true,
                &SessionKey::Main,
                Ok(ClientMsg::Send {
                    thread: thread.map(str::to_string),
                    text: "hi".into(),
                }),
            )
        };
        assert_eq!(
            send(None),
            Action::Send {
                key: SessionKey::Main,
                text: "hi".into()
            }
        );
        assert_eq!(
            send(Some("side")),
            Action::Send {
                key: SessionKey::Thread(ThreadName::parse("side").unwrap()),
                text: "hi".into()
            }
        );
        assert!(matches!(send(Some("../x")), Action::Refuse(_)));
        assert!(matches!(send(Some("main")), Action::Refuse(_)));
        assert_eq!(
            classify(
                false,
                &SessionKey::Main,
                Ok(ClientMsg::Send {
                    thread: None,
                    text: "hi".into()
                })
            ),
            Action::Refuse("unexpected message before hello".into())
        );
    }

    #[test]
    fn switch_changes_the_send_key() {
        let side = SessionKey::Thread(ThreadName::parse("side").unwrap());
        let send = |attached: &SessionKey| {
            classify(
                true,
                attached,
                Ok(ClientMsg::Send {
                    thread: None,
                    text: "hi".into(),
                }),
            )
        };
        let switch = classify(
            true,
            &SessionKey::Main,
            Ok(ClientMsg::Thread(ThreadCmd::Switch {
                name: "side".into(),
            })),
        );
        assert_eq!(switch, Action::Attach(side.clone()));
        assert_eq!(
            send(&side),
            Action::Send {
                key: side,
                text: "hi".into()
            }
        );
    }

    #[test]
    fn refusal_keeps_the_connection_open() {
        let send = Action::Send {
            key: SessionKey::Main,
            text: "hi".into(),
        };
        assert!(!closes_on_error(&send));
        assert!(!closes_on_error(&Action::Thread(ThreadOp::Ls)));
        assert!(closes_on_error(&Action::Hello(1)));
        assert!(closes_on_error(&Action::Refuse("bad".into())));
    }

    #[test]
    fn permit_maps_run_and_refusals() {
        assert_eq!(
            permit(&[Effect::Run(SessionKey::Main)]),
            Ok(SessionKey::Main)
        );
        assert_eq!(
            permit(&[Effect::Refuse("down".into())]),
            Err("down".to_string())
        );
    }

    #[test]
    fn classify_gates_on_hello() {
        assert_eq!(
            classify(false, &SessionKey::Main, Ok(ClientMsg::Hello { proto: 1 })),
            Action::Hello(1)
        );
        assert_eq!(
            classify(false, &SessionKey::Main, Ok(ClientMsg::Shutdown)),
            Action::Refuse("unexpected message before hello".into())
        );
        assert_eq!(
            classify(true, &SessionKey::Main, Ok(ClientMsg::Shutdown)),
            Action::Shutdown
        );
        assert_eq!(
            classify(true, &SessionKey::Main, Err("bad".into())),
            Action::Refuse("bad".into())
        );
    }
}
