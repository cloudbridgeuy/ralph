use super::clients::{write_frame, Writer};
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
use ralph_core::worker::HANDOFF_TIMEOUT;
use ralph_server::chat::serve_proto::{
    decode_frame, hello_reply, ClientMsg, ServerMsg, ThreadRequest,
};
use ralph_server::warn::warn;
use ralph_server::workers::answer_permission as answer_worker_permission;
use std::cell::RefCell;
use std::io::{BufRead, BufReader, Read};
use std::os::unix::net::UnixStream;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const READ_TIMEOUT: Duration = Duration::from_secs(30);
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, PartialEq, Eq)]
struct Answer {
    key: SessionKey,
    worker: String,
    request: String,
    option: String,
}

#[derive(Debug, PartialEq, Eq)]
enum Action {
    Hello(u32),
    Send { key: SessionKey, text: String },
    Cancel(SessionKey),
    Status(SessionKey),
    Thread(ThreadOp),
    Answer(Answer),
    Attach(SessionKey),
    Shutdown,
    Refuse(String),
}

pub(super) fn handle(stream: UnixStream, host: &Host) -> std::io::Result<()> {
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    let reader = BufReader::new(stream.try_clone()?);
    let writer: Writer = Arc::new(Mutex::new(stream));
    let mut client = None;
    let result = converse(reader, &writer, host, &mut client);
    if let Some(client) = client {
        host.clients.detach(client);
    }
    result
}

fn converse(
    mut reader: BufReader<UnixStream>,
    writer: &Writer,
    host: &Host,
    client: &mut Option<u64>,
) -> std::io::Result<()> {
    let mut greeted = false;
    let mut attached = SessionKey::Main;
    while let Some(line) = read_frame(&mut reader)? {
        let frame = decode_frame::<ClientMsg>(line.trim_end());
        let id = frame.as_ref().map_or(0, |envelope| envelope.id);
        let action = classify(greeted, &attached, frame.map(|envelope| envelope.body));
        let closes = closes_on_error(&action);
        let mut after = Vec::new();
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
            Action::Send { key, text } => {
                let (reply, effects) = send(host, &key, &text, id, writer);
                after = effects;
                reply
            }
            Action::Cancel(key) => cancel(host, &key),
            Action::Status(key) => status(host, &key),
            Action::Thread(op) => thread(host, op),
            Action::Answer(answer) => answer_permission(host, &answer),
            Action::Attach(key) => {
                attached = key;
                ServerMsg::Done {
                    message: format!("attached to {attached}"),
                }
            }
        };
        if matches!(reply, ServerMsg::Hello { .. }) && client.is_none() {
            *client = Some(host.clients.greet(writer, id, &reply)?);
        } else {
            write_frame(writer, id, &reply)?;
        }
        if let Err(e) = drive(host, after) {
            warn(format!("ralph serve: close after a turn failed: {e}"));
        }
        if closes && matches!(reply, ServerMsg::Error { .. }) {
            return Ok(());
        }
        let was = std::mem::replace(&mut greeted, true);
        if let Some(timeout) = timeout_change(was, greeted) {
            reader.get_ref().set_read_timeout(timeout)?;
        }
    }
    Ok(())
}

fn cancel(host: &Host, key: &SessionKey) -> ServerMsg {
    let result = step(&host.supervisor, Input::Cancel(key.clone()))
        .and_then(|effects| permit_open(&effects).map(|()| effects))
        .and_then(|effects| drive(host, effects).map_err(|e| e.to_string()));
    match result {
        Ok(()) => ServerMsg::Done {
            message: format!("cancelled {key}"),
        },
        Err(message) => ServerMsg::Error { message },
    }
}

fn status(host: &Host, key: &SessionKey) -> ServerMsg {
    host.sessions
        .get(key)
        .map(|open| open.status())
        .unwrap_or_else(|| ServerMsg::Error {
            message: format!("{key} is not open"),
        })
}

fn answer_permission(host: &Host, answer: &Answer) -> ServerMsg {
    let Some(store) = host.sessions.store(&answer.key) else {
        return ServerMsg::Error {
            message: format!("{} is not open", answer.key),
        };
    };
    match answer_worker_permission(
        &store,
        HANDOFF_TIMEOUT,
        &answer.worker,
        &answer.request,
        &answer.option,
    ) {
        Ok(message) => ServerMsg::Done { message },
        Err(e) => ServerMsg::Error {
            message: e.to_string(),
        },
    }
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
    permit_open(&step(
        &host.supervisor,
        Input::Thread(ThreadOp::Open(opts.clone())),
    )?)?;
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
        mid_turn: false,
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

fn send(
    host: &Host,
    key: &SessionKey,
    text: &str,
    id: u64,
    writer: &Writer,
) -> (ServerMsg, Vec<Effect>) {
    let mut after = Vec::new();
    let run = step(&host.supervisor, Input::Send(key.clone())).and_then(|effects| permit(&effects));
    let result = run.and_then(|key| {
        let outcome = run_turn(host, &key, text, id, writer);
        after = step(&host.supervisor, Input::TurnDone(key))?;
        outcome
    });
    let reply = match result {
        Ok(()) => ServerMsg::TurnDone,
        Err(message) => ServerMsg::Error { message },
    };
    (reply, after)
}

fn run_turn(
    host: &Host,
    key: &SessionKey,
    text: &str,
    id: u64,
    writer: &Writer,
) -> Result<(), String> {
    let open = host
        .sessions
        .get(key)
        .ok_or_else(|| format!("{key} is not open"))?;
    open.sink.bind(id, Arc::clone(writer));
    let outcome = open.turn(text);
    open.sink.unbind();
    outcome.map(|_| ()).map_err(|e| e.to_string())
}

fn closes_on_error(action: &Action) -> bool {
    !matches!(
        action,
        Action::Send { .. }
            | Action::Cancel(_)
            | Action::Status(_)
            | Action::Thread(_)
            | Action::Answer(_)
            | Action::Attach(_)
    )
}

fn timeout_change(was: bool, now: bool) -> Option<Option<Duration>> {
    (!was && now).then_some(None)
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
        Ok(ClientMsg::Cancel { thread }) => match session_key(thread.as_deref(), attached) {
            Ok(key) => Action::Cancel(key),
            Err(message) => Action::Refuse(message),
        },
        Ok(ClientMsg::Status { thread }) => match session_key(thread.as_deref(), attached) {
            Ok(key) => Action::Status(key),
            Err(message) => Action::Refuse(message),
        },
        Ok(ClientMsg::Thread(cmd)) => match cmd.parse() {
            Ok(ThreadRequest::Op(op)) => Action::Thread(op),
            Ok(ThreadRequest::Attach(key)) => Action::Attach(key),
            Err(message) => Action::Refuse(message),
        },
        Ok(ClientMsg::Answer {
            thread,
            worker,
            request,
            option,
        }) => match session_key(thread.as_deref(), attached) {
            Ok(key) => Action::Answer(Answer {
                key,
                worker,
                request,
                option,
            }),
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
            answer(&[Effect::Answer { proto: 2 }]),
            ServerMsg::Hello { proto: 2 }
        ));
        assert!(matches!(
            answer(&[Effect::Answer { proto: 1 }]),
            ServerMsg::Error { .. }
        ));
        let ServerMsg::Error { message } = answer(&[Effect::Refuse("down".into())]) else {
            panic!("expected error");
        };
        assert_eq!(message, "down");
    }

    #[test]
    fn the_read_timeout_changes_only_when_a_connection_is_greeted() {
        assert_eq!(timeout_change(false, true), Some(None));
        assert_eq!(timeout_change(true, true), None);
        assert_eq!(timeout_change(false, false), None);
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
    fn classify_routes_cancel_and_status_to_a_session_key() {
        let side = SessionKey::Thread(ThreadName::parse("side").unwrap());
        let cancel = |thread: Option<&str>| {
            classify(
                true,
                &side,
                Ok(ClientMsg::Cancel {
                    thread: thread.map(str::to_string),
                }),
            )
        };
        assert_eq!(cancel(None), Action::Cancel(side.clone()));
        assert_eq!(
            cancel(Some("main")),
            Action::Refuse(ThreadName::parse("main").unwrap_err())
        );
        let status = classify(true, &side, Ok(ClientMsg::Status { thread: None }));
        assert_eq!(status, Action::Status(side));
        assert!(!closes_on_error(&Action::Cancel(SessionKey::Main)));
        assert!(!closes_on_error(&Action::Status(SessionKey::Main)));
    }

    #[test]
    fn classify_routes_an_answer_to_a_session_key() {
        let answer = |thread: Option<&str>| {
            classify(
                true,
                &SessionKey::Main,
                Ok(ClientMsg::Answer {
                    thread: thread.map(str::to_string),
                    worker: "w1".into(),
                    request: "per_1".into(),
                    option: "once".into(),
                }),
            )
        };
        let expected = |key| {
            Action::Answer(Answer {
                key,
                worker: "w1".into(),
                request: "per_1".into(),
                option: "once".into(),
            })
        };
        assert_eq!(answer(None), expected(SessionKey::Main));
        assert_eq!(
            answer(Some("side")),
            expected(SessionKey::Thread(ThreadName::parse("side").unwrap()))
        );
        assert!(matches!(answer(Some("main")), Action::Refuse(_)));
        assert!(!closes_on_error(&expected(SessionKey::Main)));
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
