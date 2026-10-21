mod proxy;
mod store;

use crate::cli::{WireArgs, WireProvider};
use chrono::Utc;
use ralph_core::wire::provider::{provider, ProviderId};
use ralph_core::wire::record::{SessionRecord, SCHEMA_VERSION};
use ralph_core::wire::session::session_id;
use std::collections::hash_map::RandomState;
use std::error::Error;
use std::hash::{BuildHasher, Hasher};
use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::thread;

impl From<WireProvider> for ProviderId {
    fn from(value: WireProvider) -> Self {
        match value {
            WireProvider::Anthropic => ProviderId::Anthropic,
        }
    }
}

fn random_suffix() -> u32 {
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u128(Utc::now().timestamp_nanos_opt().unwrap_or_default() as u128);
    hasher.write_u32(std::process::id());
    hasher.finish() as u32
}

pub fn run(args: WireArgs) -> Result<(), Box<dyn Error>> {
    let provider = provider(args.provider.into());
    let upstream = args
        .upstream
        .unwrap_or_else(|| provider.default_upstream().to_string());
    let server = tiny_http::Server::http(("127.0.0.1", args.port)).map_err(|e| e.to_string())?;
    let listen = server.server_addr().to_string();
    let started_at = Utc::now();
    let root = args
        .dir
        .unwrap_or_else(|| std::env::temp_dir().join("ralph-wire"));
    let dir: PathBuf = root.join(session_id(started_at, random_suffix()));
    let calls = store::open_session(
        &dir,
        &SessionRecord {
            schema_version: SCHEMA_VERSION,
            ralph_version: env!("CARGO_PKG_VERSION").to_string(),
            provider: provider.id(),
            upstream: upstream.clone(),
            listen: listen.clone(),
            started_at,
        },
    )?;
    println!("ralph wire listening on http://{listen} -> {upstream}");
    println!("session: {}", dir.display());
    println!(
        "warning: prompts and responses are recorded in plaintext in {}",
        dir.display()
    );
    println!("export {}=http://{listen}", provider.base_url_env());
    let (records, receiver) = mpsc::channel();
    thread::spawn(move || store::write_loop(receiver, calls));
    let proxy = Arc::new(proxy::Proxy::new(
        provider,
        upstream,
        args.responses,
        records,
    ));
    for request in server.incoming_requests() {
        let proxy = Arc::clone(&proxy);
        thread::spawn(move || proxy.handle(request));
    }
    Ok(())
}
