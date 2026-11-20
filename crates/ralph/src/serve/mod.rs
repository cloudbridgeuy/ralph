mod connection;
mod open;
mod sink;

use crate::chat::STORE_ENV;
use crate::cli::ServeArgs;
use crate::signal;
use open::{open_session, Sessions};
use ralph_core::chat_process::SessionKey;
use ralph_core::chat_socket::{runtime_root, serve_socket_path, StoreKey};
use ralph_core::serve::{serve_refusal, serve_store_gate};
use ralph_core::serve_supervisor::{Effect, Input, Supervisor};
use ralph_server::chat::claim::ChatClaim;
use ralph_server::chat::memory::Store;
use ralph_server::chat::serve_socket::ServeSocket;
use ralph_server::warn::warn;
use ralph_server::wire::store::current_uid;
use ralph_server::workers::canonical_store;
use std::collections::VecDeque;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const ACCEPT_RETRY: Duration = Duration::from_millis(50);
const POISONED: &str = "ralph serve: internal state unavailable";

struct Host {
    supervisor: Mutex<Supervisor>,
    sessions: Sessions,
    store_dir: PathBuf,
    stop: AtomicBool,
}

pub fn run(args: ServeArgs) -> Result<(), Box<dyn std::error::Error>> {
    serve_store_gate(std::env::var(STORE_ENV).ok().as_deref())?;
    signal::init_serve()?;
    let requested = std::env::current_dir()?.join(&args.store);
    std::fs::create_dir_all(&requested)?;
    let store_dir = canonical_store(requested);
    ralph_server::config::set_store_layer(&store_dir);
    let store = Store::new(store_dir.clone());
    let (supervisor, effects) = Supervisor::start();
    let host = Arc::new(Host {
        supervisor: Mutex::new(supervisor),
        sessions: Sessions::default(),
        store_dir: store_dir.clone(),
        stop: AtomicBool::new(false),
    });
    let mut claim = None;
    drive(&host, &store, &mut claim, effects)?;
    let root = runtime_root(current_uid());
    let path = serve_socket_path(&root, &StoreKey::of(&store_dir));
    let socket = ServeSocket::bind(&root, &path)?;
    eprintln!("ralph serve: listening on {}", path.display());
    loop {
        if signal::is_terminated() || host.stop.load(Ordering::SeqCst) {
            let effects = step(&host.supervisor, Input::Stop)?;
            if drive(&host, &store, &mut claim, effects)? {
                break;
            }
        }
        match socket.accept() {
            Ok(Some(stream)) => spawn_connection(stream, &host),
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
    host: &Host,
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
                host.sessions
                    .insert(SessionKey::Main, open_session(store.dir())?);
            }
            Effect::Close(SessionKey::Main) => {
                if let Some(open) = host.sessions.remove(&SessionKey::Main) {
                    open.end();
                }
                claim.take();
                queue.extend(step(&host.supervisor, Input::Closed(SessionKey::Main))?);
            }
            Effect::Exit => return Ok(true),
            _ => {}
        }
    }
    Ok(false)
}

fn spawn_connection(stream: UnixStream, host: &Arc<Host>) {
    let host = Arc::clone(host);
    let spawned = std::thread::Builder::new()
        .name("serve-connection".to_string())
        .spawn(move || {
            if let Err(e) = connection::handle(stream, &host) {
                warn(format!("ralph serve: connection ended: {e}"));
            }
        });
    if let Err(e) = spawned {
        warn(format!(
            "ralph serve: cannot start a connection thread: {e}"
        ));
    }
}
