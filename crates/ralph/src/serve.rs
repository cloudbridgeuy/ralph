use crate::chat::STORE_ENV;
use crate::cli::ServeArgs;
use crate::signal;
use ralph_core::chat_process::SessionKey;
use ralph_core::chat_socket::{runtime_root, serve_socket_path, StoreKey, MAX_FRAME_BYTES};
use ralph_core::serve::{serve_refusal, serve_store_gate};
use ralph_core::serve_supervisor::{Effect, Input, Supervisor};
use ralph_server::chat::claim::ChatClaim;
use ralph_server::chat::memory::Store;
use ralph_server::chat::serve_proto::{
    decode_frame, encode_frame, hello_reply, ClientMsg, ServerMsg,
};
use ralph_server::chat::serve_socket::ServeSocket;
use ralph_server::warn::warn;
use ralph_server::wire::store::current_uid;
use ralph_server::workers::canonical_store;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const ACCEPT_RETRY: Duration = Duration::from_millis(50);
const READ_TIMEOUT: Duration = Duration::from_secs(30);
const POISONED: &str = "ralph serve: internal state unavailable";

type Shared = Arc<Mutex<Supervisor>>;

#[derive(Debug, PartialEq, Eq)]
enum Action {
    Hello(u32),
    Shutdown,
    Refuse(String),
}

pub fn run(args: ServeArgs) -> Result<(), Box<dyn std::error::Error>> {
    serve_store_gate(std::env::var(STORE_ENV).ok().as_deref())?;
    signal::init_serve()?;
    let requested = std::env::current_dir()?.join(&args.store);
    std::fs::create_dir_all(&requested)?;
    let store_dir = canonical_store(requested);
    let store = Store::new(store_dir.clone());
    let (supervisor, effects) = Supervisor::start();
    let supervisor: Shared = Arc::new(Mutex::new(supervisor));
    let mut claim = None;
    drive(&supervisor, &store, &mut claim, effects)?;
    let root = runtime_root(current_uid());
    let path = serve_socket_path(&root, &StoreKey::of(&store_dir));
    let socket = ServeSocket::bind(&root, &path)?;
    eprintln!("ralph serve: listening on {}", path.display());
    let stop = Arc::new(AtomicBool::new(false));
    loop {
        if signal::is_terminated() || stop.load(Ordering::SeqCst) {
            let effects = step(&supervisor, Input::Stop)?;
            if drive(&supervisor, &store, &mut claim, effects)? {
                break;
            }
        }
        match socket.accept() {
            Ok(Some(stream)) => spawn_connection(stream, &supervisor, &stop),
            Ok(None) => std::thread::sleep(ACCEPT_RETRY),
            Err(e) => {
                warn(format!("ralph serve: accept failed: {e}"));
                std::thread::sleep(ACCEPT_RETRY);
            }
        }
    }
    Ok(())
}

fn step(supervisor: &Mutex<Supervisor>, input: Input) -> Result<Vec<Effect>, String> {
    supervisor
        .lock()
        .map(|mut supervisor| supervisor.step(input))
        .map_err(|_| POISONED.to_string())
}

fn drive(
    supervisor: &Mutex<Supervisor>,
    store: &Store,
    claim: &mut Option<ChatClaim>,
    effects: Vec<Effect>,
) -> Result<bool, Box<dyn std::error::Error>> {
    let mut queue = VecDeque::from(effects);
    while let Some(effect) = queue.pop_front() {
        match effect {
            Effect::Open(SessionKey::Main) => {
                let held = store
                    .claim_chat()?
                    .map_err(|holder| serve_refusal(store.dir(), holder))?;
                *claim = Some(held);
            }
            Effect::Close(SessionKey::Main) => {
                claim.take();
                queue.extend(step(supervisor, Input::Closed(SessionKey::Main))?);
            }
            Effect::Exit => return Ok(true),
            _ => {}
        }
    }
    Ok(false)
}

fn spawn_connection(stream: UnixStream, supervisor: &Shared, stop: &Arc<AtomicBool>) {
    let supervisor = Arc::clone(supervisor);
    let stop = Arc::clone(stop);
    let spawned = std::thread::Builder::new()
        .name("serve-connection".to_string())
        .spawn(move || {
            if let Err(e) = handle(stream, &supervisor, &stop) {
                warn(format!("ralph serve: connection ended: {e}"));
            }
        });
    if let Err(e) = spawned {
        warn(format!(
            "ralph serve: cannot start a connection thread: {e}"
        ));
    }
}

fn handle(
    stream: UnixStream,
    supervisor: &Mutex<Supervisor>,
    stop: &AtomicBool,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    let mut greeted = false;
    while let Some(line) = read_frame(&mut reader)? {
        let frame = decode_frame::<ClientMsg>(line.trim_end());
        let id = frame.as_ref().map_or(0, |envelope| envelope.id);
        let reply = match classify(greeted, frame.map(|envelope| envelope.body)) {
            Action::Shutdown => {
                stop.store(true, Ordering::SeqCst);
                return Ok(());
            }
            Action::Refuse(message) => ServerMsg::Error { message },
            Action::Hello(proto) => match step(supervisor, Input::Hello { proto }) {
                Ok(effects) => answer(&effects),
                Err(message) => ServerMsg::Error { message },
            },
        };
        writer.write_all(encode_frame(id, &reply).as_bytes())?;
        if matches!(reply, ServerMsg::Error { .. }) {
            return Ok(());
        }
        greeted = true;
    }
    Ok(())
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
