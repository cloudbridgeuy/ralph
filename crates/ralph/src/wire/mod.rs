mod proxy;
mod shutdown;
mod store;

use crate::cli::{
    WireArgs, WireCommand, WireDiffArgs, WireLsArgs, WireProvider, WireSessionsArgs, WireShowArgs,
};
use crate::config::AppConfig;
use chrono::Utc;
use ralph_core::wire::diff::{check_pair, diff_table, diff_view, pick_base, DiffInput};
use ralph_core::wire::provider::{provider, ProviderId};
use ralph_core::wire::read::{find_call, pick_session, Calls};
use ralph_core::wire::record::{SessionRecord, SCHEMA_VERSION};
use ralph_core::wire::session::session_id;
use ralph_core::wire::view::{
    ls_table, ls_view, session_rows, sessions_table, show_table, show_view, Pricing,
};
use std::collections::hash_map::RandomState;
use std::error::Error;
use std::hash::{BuildHasher, Hasher};
use std::io;
use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use self::shutdown::{drain_action, DrainAction, SHUTDOWN_DRAIN};

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

fn root_or_default(dir: Option<PathBuf>) -> PathBuf {
    dir.unwrap_or_else(|| std::env::temp_dir().join("ralph-wire"))
}

fn load_session(
    dir: Option<PathBuf>,
    requested: Option<String>,
) -> Result<(String, SessionRecord, Calls), Box<dyn Error>> {
    let root = root_or_default(dir);
    let ids =
        store::list_sessions(&root).map_err(|error| format!("{}: {error}", root.display()))?;
    let id = pick_session(ids, requested, &root.display().to_string())?;
    let (session, calls) = store::read_session(&root, &id)?;
    if calls.torn_tail {
        eprintln!(
            "note: {}: ignored incomplete last line (session in progress or interrupted)",
            root.join(&id).join("calls.jsonl").display()
        );
    }
    Ok((id, session, calls))
}

fn ls(args: WireLsArgs) -> Result<(), Box<dyn Error>> {
    let (id, session, calls) = load_session(args.dir, args.session)?;
    let config = AppConfig::load()?;
    let provider = provider(session.provider);
    let pricing = Pricing {
        provider: provider.as_ref(),
        overrides: &config.wire.prices,
    };
    let view = ls_view(&calls.records, args.all, &pricing);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&view)?);
    } else {
        println!("session: {id}");
        println!("{}", ls_table(&view));
    }
    Ok(())
}

fn show(args: WireShowArgs) -> Result<(), Box<dyn Error>> {
    let (id, session, calls) = load_session(args.dir, args.session)?;
    let call = find_call(&calls.records, args.n, &id)?;
    let view = show_view(call, &id, provider(session.provider).as_ref());
    if args.json {
        println!("{}", serde_json::to_string_pretty(&view)?);
    } else {
        println!("{}", show_table(&view));
    }
    Ok(())
}

fn diff(args: WireDiffArgs) -> Result<(), Box<dyn Error>> {
    let (id, session, calls) = load_session(args.dir, args.session)?;
    let provider = provider(session.provider);
    let (a, b) = match args.b {
        Some(b) => (
            Some(find_call(&calls.records, args.a, &id)?),
            find_call(&calls.records, b, &id)?,
        ),
        None => (
            pick_base(&calls.records, args.a, &id, provider.as_ref())?,
            find_call(&calls.records, args.a, &id)?,
        ),
    };
    check_pair(a, b)?;
    let view = diff_view(&DiffInput {
        a,
        b,
        session: &calls.records,
        provider: provider.as_ref(),
        base_picked: args.b.is_none(),
    });
    if args.json {
        println!("{}", serde_json::to_string_pretty(&view)?);
    } else {
        println!("session: {id}");
        println!("{}", diff_table(&view));
    }
    Ok(())
}

fn sessions(args: WireSessionsArgs) -> Result<(), Box<dyn Error>> {
    let root = root_or_default(args.dir);
    let reads = store::list_sessions(&root)
        .or_else(|error| match error.kind() {
            io::ErrorKind::NotFound => Ok(vec![]),
            _ => Err(error),
        })?
        .into_iter()
        .map(|id| {
            let read = store::read_session(&root, &id);
            (id, read)
        })
        .collect();
    let (rows, warnings) = session_rows(reads);
    for warning in warnings {
        eprintln!("ralph wire: {warning}");
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else {
        println!("{}", sessions_table(&rows));
    }
    Ok(())
}

pub fn run(args: WireArgs) -> Result<(), Box<dyn Error>> {
    match args.command {
        Some(WireCommand::Ls(ls_args)) => ls(ls_args),
        Some(WireCommand::Show(show_args)) => show(show_args),
        Some(WireCommand::Diff(diff_args)) => diff(diff_args),
        Some(WireCommand::Sessions(sessions_args)) => sessions(sessions_args),
        None => proxy(args),
    }
}

fn proxy(args: WireArgs) -> Result<(), Box<dyn Error>> {
    crate::signal::init()?;
    let provider = provider(args.provider.into());
    let upstream = args
        .upstream
        .unwrap_or_else(|| provider.default_upstream().to_string());
    let server = tiny_http::Server::http(("127.0.0.1", args.port)).map_err(|e| e.to_string())?;
    let listen = server.server_addr().to_string();
    let started_at = Utc::now();
    let root = root_or_default(args.dir);
    store::ensure_store_root(&root)?;
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
    let writer = thread::spawn(move || store::write_loop(receiver, calls));
    let proxy = Arc::new(proxy::Proxy::new(
        provider,
        upstream,
        args.responses,
        records,
    ));
    let accept_wait = Duration::from_millis(100);
    loop {
        if crate::signal::is_interrupted() {
            break;
        }
        match server.recv_timeout(accept_wait) {
            Ok(Some(request)) => {
                let proxy = Arc::clone(&proxy);
                thread::spawn(move || proxy.handle(request));
            }
            Ok(None) => {}
            Err(error) => {
                eprintln!("ralph wire: accept failed: {error}");
                break;
            }
        }
    }
    proxy.stop_accepting();
    let drain_started = Instant::now();
    loop {
        let open = proxy.open_len();
        if drain_action(drain_started.elapsed(), SHUTDOWN_DRAIN, open) == DrainAction::Finish {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    proxy.close_open();
    if writer.join().is_err() {
        eprintln!("ralph wire: recorder stopped");
    }
    Ok(())
}
