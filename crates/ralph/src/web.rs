use crate::chat::session::store_dir;
use crate::cli::WebArgs;
use ralph_api::{allowed_hosts, bind, serve, ApiConfig, BindPolicy, Token};
use ralph_server::workers::canonical_store;
use std::error::Error;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

const HOST_NAMES: [&str; 2] = ["127.0.0.1", "localhost"];

pub fn run(args: WebArgs) -> Result<(), Box<dyn Error>> {
    let store = store_dir(args.store)?;
    if !store.is_dir() {
        return Err(format!("store not found: {}", store.display()).into());
    }
    let store = canonical_store(store);
    crate::signal::init_serve()?;
    let token = Token::generate()?;
    let requested = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), args.port);
    let server = bind(requested, &BindPolicy::loopback())?;
    let port = server
        .server_addr()
        .to_ip()
        .ok_or("the server is not listening on an IP address")?
        .port();
    let config = ApiConfig {
        store,
        allowed_hosts: allowed_hosts(&HOST_NAMES, port),
    };
    println!(
        "ralph web listening on http://127.0.0.1:{port}/?token={}",
        token.as_str()
    );
    println!("store: {}", config.store.display());
    serve(server, &config, &token, &|| crate::signal::is_terminated())?;
    Ok(())
}
