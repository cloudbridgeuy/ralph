use crate::chat::session::store_dir;
use crate::cli::WebArgs;
use crate::web_tokens::render_tokens;
use ralph_api::{
    allowed_hosts, bind, bind_hosts, cookie_name, serve, ApiConfig, ApiError, BindPolicy, Token,
};
use ralph_server::config::{config_path, AppConfig};
use ralph_server::workers::canonical_store;
use std::error::Error;
use std::net::SocketAddr;
use std::path::PathBuf;

pub fn run(args: WebArgs) -> Result<(), Box<dyn Error>> {
    let store = store_dir(args.store)?;
    if !store.is_dir() {
        return Err(format!("store not found: {}", store.display()).into());
    }
    let store = canonical_store(store);
    let app = AppConfig::load_layers(&config_path(), Some(&store.join("config.toml")))?;
    let tokens_css = render_tokens(&app)?;
    crate::signal::init_serve()?;
    let token = Token::generate()?;
    let policy = BindPolicy::loopback();
    let ip = policy
        .primary()
        .ok_or("the bind policy allows no address")?;
    let (port, source) = resolve_port(args.port, app.web.port);
    let server = bind(SocketAddr::new(ip, port), &policy).map_err(|error| match error {
        ApiError::Listen { reason, .. } => port_conflict_message(port, source, &reason).into(),
        other => Box::<dyn Error>::from(other),
    })?;
    let bound = server
        .server_addr()
        .to_ip()
        .ok_or("the server is not listening on an IP address")?;
    let config = api_config(store, bound, crate::version::current(), tokens_css);
    println!(
        "ralph web listening on {}",
        listen_url(bound, token.as_str())
    );
    println!("store: {}", config.store.display());
    serve(server, &config, &token, &|| crate::signal::is_terminated())?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PortSource {
    Flag,
    Config,
    Ephemeral,
}

fn resolve_port(cli: Option<u16>, cfg: Option<u16>) -> (u16, PortSource) {
    match (cli, cfg) {
        (Some(0), _) | (None, None | Some(0)) => (0, PortSource::Ephemeral),
        (Some(port), _) => (port, PortSource::Flag),
        (None, Some(port)) => (port, PortSource::Config),
    }
}

fn port_conflict_message(port: u16, source: PortSource, reason: &str) -> String {
    let origin = match source {
        PortSource::Flag => "--port",
        PortSource::Config => "[web] port",
        PortSource::Ephemeral => "ephemeral",
    };
    format!("port {port} in use (from {origin}): {reason}")
}

fn api_config(store: PathBuf, bound: SocketAddr, version: &str, tokens_css: String) -> ApiConfig {
    ApiConfig {
        store,
        allowed_hosts: allowed_hosts(&bind_hosts(bound.ip()), bound.port()),
        version: version.to_string(),
        tokens_css,
        cookie_name: cookie_name(bound.port()),
    }
}

fn listen_url(bound: SocketAddr, token: &str) -> String {
    format!("http://{bound}/?token={token}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(text: &str) -> SocketAddr {
        text.parse().unwrap()
    }

    #[test]
    fn api_config_carries_the_given_version_and_hosts() {
        let config = api_config(
            PathBuf::from("/s"),
            addr("127.0.0.1:4000"),
            "1.2.3-dev+abc",
            "css".to_string(),
        );
        assert_eq!(config.version, "1.2.3-dev+abc");
        assert_eq!(config.store, PathBuf::from("/s"));
        assert_eq!(config.tokens_css, "css");
        assert_eq!(config.cookie_name, "ralph_token_4000");
        assert_eq!(
            config.allowed_hosts,
            vec!["127.0.0.1:4000".to_string(), "localhost:4000".to_string()]
        );
    }

    #[test]
    fn api_config_follows_the_bound_ip() {
        let config = api_config(PathBuf::from("/s"), addr("[::1]:4000"), "v", String::new());
        assert_eq!(
            config.allowed_hosts,
            vec!["[::1]:4000".to_string(), "localhost:4000".to_string()]
        );
    }

    #[test]
    fn listen_url_points_at_the_shell_with_the_token() {
        assert_eq!(
            listen_url(addr("127.0.0.1:4000"), "abc"),
            "http://127.0.0.1:4000/?token=abc"
        );
    }

    #[test]
    fn listen_url_brackets_ipv6() {
        assert_eq!(
            listen_url(addr("[::1]:4000"), "abc"),
            "http://[::1]:4000/?token=abc"
        );
    }

    #[test]
    fn resolve_port_precedence() {
        let cases = [
            (Some(80), Some(90), (80, PortSource::Flag)),
            (None, Some(90), (90, PortSource::Config)),
            (Some(0), Some(90), (0, PortSource::Ephemeral)),
            (None, None, (0, PortSource::Ephemeral)),
            (None, Some(0), (0, PortSource::Ephemeral)),
            (Some(80), None, (80, PortSource::Flag)),
        ];
        for (cli, cfg, want) in cases {
            assert_eq!(resolve_port(cli, cfg), want, "{cli:?} {cfg:?}");
        }
    }

    #[test]
    fn conflict_message_names_the_source() {
        assert_eq!(
            port_conflict_message(80, PortSource::Flag, "denied"),
            "port 80 in use (from --port): denied"
        );
        assert_eq!(
            port_conflict_message(81, PortSource::Config, "denied"),
            "port 81 in use (from [web] port): denied"
        );
    }
}
