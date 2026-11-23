mod blocked;
mod clients;
mod connection;
mod open;
mod sink;

use crate::chat::thread::open::model_args;
use crate::chat::thread::{claim_closed, config_root, thread_dir};
use crate::chat::STORE_ENV;
use crate::cli::ServeArgs;
use crate::signal;
use blocked::read_blocked;
use clients::Clients;
use open::{open_session, Sessions};
use ralph_core::chat_process::SessionKey;
use ralph_core::chat_socket::{serve_socket_path, StoreKey};
use ralph_core::chat_thread::ThreadName;
use ralph_core::serve::{serve_refusal, serve_store_gate};
use ralph_core::serve_supervisor::{Effect, Input, Supervisor};
use ralph_server::chat::claim::ChatClaim;
use ralph_server::chat::memory::Store;
use ralph_server::chat::serve_proto::{blocked_pushes, BlockedMap};
use ralph_server::chat::serve_socket::ServeSocket;
use ralph_server::warn::warn;
use ralph_server::wire::store::default_runtime_root;
use ralph_server::workers::canonical_store;
use std::collections::VecDeque;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const DRAIN_BOUND: Duration = Duration::from_secs(5);
const ACCEPT_RETRY: Duration = Duration::from_millis(50);
const BLOCKED_POLL: Duration = Duration::from_secs(1);
const POISONED: &str = "ralph serve: internal state unavailable";

struct Host {
    supervisor: Mutex<Supervisor>,
    sessions: Sessions,
    clients: Arc<Clients>,
    store_dir: PathBuf,
    claim: Mutex<Option<ChatClaim>>,
    stop: AtomicBool,
    exited: AtomicBool,
}

pub fn run(args: ServeArgs) -> Result<(), Box<dyn std::error::Error>> {
    serve_store_gate(std::env::var(STORE_ENV).ok().as_deref())?;
    signal::init_serve()?;
    let requested = std::env::current_dir()?.join(&args.store);
    std::fs::create_dir_all(&requested)?;
    let store_dir = canonical_store(requested);
    ralph_server::config::set_store_layer(&store_dir);
    let (supervisor, effects) = Supervisor::start();
    let host = Arc::new(Host {
        supervisor: Mutex::new(supervisor),
        sessions: Sessions::default(),
        clients: Arc::default(),
        store_dir: store_dir.clone(),
        claim: Mutex::new(None),
        stop: AtomicBool::new(false),
        exited: AtomicBool::new(false),
    });
    drive(&host, effects)?;
    let root = default_runtime_root()?;
    let path = serve_socket_path(&root, &StoreKey::of(&store_dir));
    let socket = ServeSocket::bind(&root, &path)?;
    eprintln!("ralph serve: listening on {}", path.display());
    let mut draining_since = None;
    let mut seen = BlockedMap::new();
    let mut polled = Instant::now();
    loop {
        if polled.elapsed() >= BLOCKED_POLL {
            polled = Instant::now();
            let now = read_blocked(&host.sessions.stores());
            blocked_pushes(&seen, &now)
                .iter()
                .for_each(|push| host.clients.push(push));
            seen = now;
        }
        if signal::is_terminated() || host.stop.load(Ordering::SeqCst) {
            let since = draining_since.get_or_insert_with(Instant::now);
            let effects = step(&host.supervisor, Input::Stop)?;
            for effect in effects {
                if matches!(effect, Effect::Close(_)) {
                    let host = Arc::clone(&host);
                    std::thread::Builder::new()
                        .name("serve-close".into())
                        .spawn(move || {
                            if let Err(error) = drive(&host, vec![effect]) {
                                warn(format!("ralph serve: close failed: {error}"));
                            }
                        })?;
                } else {
                    drive(&host, vec![effect])?;
                }
            }
            if host.exited.load(Ordering::SeqCst) {
                break;
            }
            if since.elapsed() > DRAIN_BOUND {
                warn("ralph serve: a session did not close in time; exiting".to_string());
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

fn drive(host: &Host, effects: Vec<Effect>) -> Result<(), Box<dyn std::error::Error>> {
    let mut queue = VecDeque::from(effects);
    while let Some(effect) = queue.pop_front() {
        match effect {
            Effect::Open(SessionKey::Main) => {
                let store = Store::new(host.store_dir.clone());
                let held = store
                    .claim_chat()?
                    .map_err(|holder| serve_refusal(store.dir(), holder))?;
                *host
                    .claim
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(held);
                let key = SessionKey::Main;
                let session =
                    open_session(&key, store.dir(), Vec::new(), Arc::clone(&host.clients))?;
                host.sessions.insert(key, session);
            }
            Effect::Open(SessionKey::Thread(name)) => open_thread_session(host, &name)?,
            Effect::Close(key) => {
                if let Some(open) = host.sessions.remove(&key) {
                    open.end();
                }
                if key == SessionKey::Main {
                    host.claim
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .take();
                }
                queue.extend(step(&host.supervisor, Input::Closed(key))?);
            }
            Effect::Cancel(key) => {
                if let Some(open) = host.sessions.get(&key) {
                    open.cancel();
                }
            }
            Effect::Exit => host.exited.store(true, Ordering::SeqCst),
            _ => {}
        }
    }
    Ok(())
}

fn open_thread_session(host: &Host, name: &ThreadName) -> Result<(), Box<dyn std::error::Error>> {
    let dir = thread_dir(&config_root(&host.store_dir), name);
    let model_args = model_args(&host.store_dir, Some(name), None)?;
    let claim = claim_closed(&dir, name)?;
    let key = SessionKey::Thread(name.clone());
    let session = open_session(&key, &dir, model_args, Arc::clone(&host.clients))?.holding(claim);
    host.sessions.insert(key, session);
    Ok(())
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
