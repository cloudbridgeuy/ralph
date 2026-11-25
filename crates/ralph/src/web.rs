use crate::chat::session::store_dir;
use crate::cli::WebArgs;
use crate::web_tailscale::discover;
use crate::web_tokens::render_tokens;
use ralph_api::{
    allowed_hosts, bind, bind_hosts, cookie_name, serve, ApiConfig, ApiError, BindPolicy, Token,
};
use ralph_api::{tailnet_hosts, TailnetNames};
use ralph_server::config::{config_path, AppConfig};
use ralph_server::workers::canonical_store;
use std::error::Error;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use tiny_http::Server;

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
    let (port, source) = resolve_port(args.port, app.web.port);
    let tailnet = args.tailscale.then(discover).transpose()?;
    let policy = match &tailnet {
        Some(found) => BindPolicy::loopback_and(IpAddr::V4(found.ip))?,
        None => BindPolicy::loopback(),
    };
    let ip = policy
        .primary()
        .ok_or("the bind policy allows no address")?;
    let mut attempt = 1;
    let (server, second, bound) = loop {
        match bind_pair(ip, port, tailnet.as_ref().map(|found| found.ip), &policy) {
            Ok(pair) => break pair,
            Err(BindFailure::Second(..)) if should_retry_bind(source, attempt) => attempt += 1,
            Err(BindFailure::First(reason)) => {
                return Err(port_conflict_message(port, source, &reason).into())
            }
            Err(BindFailure::Second(bound_port, reason)) => {
                return Err(port_conflict_message(bound_port, source, &reason).into())
            }
            Err(BindFailure::Other(other)) => return Err(other),
        }
    };
    let mut config = api_config(store, bound, crate::version::current(), tokens_css);
    if let Some(found) = &tailnet {
        config.allowed_hosts = tailnet_hosts(found.ip, &found.names, bound.port());
    }
    match &tailnet {
        None => println!(
            "ralph web listening on {}",
            listen_url(bound, token.as_str())
        ),
        Some(found) => {
            println!("ralph web listening on:");
            for url in listen_urls(bound, Some((found.ip, &found.names)), token.as_str()) {
                println!("  {url}");
            }
        }
    }
    println!("store: {}", config.store.display());
    let servers = std::iter::once(server).chain(second).collect::<Vec<_>>();
    serve_all(servers, &config, &token)?;
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

const EPHEMERAL_BIND_ATTEMPTS: usize = 5;

fn should_retry_bind(source: PortSource, attempt: usize) -> bool {
    source == PortSource::Ephemeral && attempt < EPHEMERAL_BIND_ATTEMPTS
}

enum BindFailure {
    First(String),
    Second(u16, String),
    Other(Box<dyn Error>),
}

type BoundPair = (Server, Option<Server>, SocketAddr);

fn bind_pair(
    ip: IpAddr,
    port: u16,
    tailnet: Option<Ipv4Addr>,
    policy: &BindPolicy,
) -> Result<BoundPair, BindFailure> {
    let server = bind(SocketAddr::new(ip, port), policy).map_err(|error| match error {
        ApiError::Listen { reason, .. } => BindFailure::First(reason),
        other => BindFailure::Other(other.into()),
    })?;
    let bound = server
        .server_addr()
        .to_ip()
        .ok_or_else(|| BindFailure::Other("the server is not listening on an IP address".into()))?;
    let second = tailnet
        .map(|tail| {
            bind(SocketAddr::new(IpAddr::V4(tail), bound.port()), policy).map_err(|error| {
                match error {
                    ApiError::Listen { reason, .. } => BindFailure::Second(bound.port(), reason),
                    other => BindFailure::Other(other.into()),
                }
            })
        })
        .transpose()?;
    Ok((server, second, bound))
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
        global_config: config_path(),
        env: |name| std::env::var(name).ok(),
    }
}

fn listen_url(bound: SocketAddr, token: &str) -> String {
    format!("http://{bound}/?token={token}")
}

fn listen_urls(
    loopback: SocketAddr,
    tailnet: Option<(Ipv4Addr, &TailnetNames)>,
    token: &str,
) -> Vec<String> {
    let mut urls = vec![listen_url(loopback, token)];
    if let Some((ip, names)) = tailnet {
        urls.push(listen_url(
            SocketAddr::new(IpAddr::V4(ip), loopback.port()),
            token,
        ));
        if let Some(fqdn) = &names.fqdn {
            urls.push(format!("http://{fqdn}:{}/?token={token}", loopback.port()));
        }
    }
    urls
}

fn should_stop(terminated: bool, failed: bool) -> bool {
    terminated || failed
}

fn serve_all(servers: Vec<Server>, config: &ApiConfig, token: &Token) -> Result<(), ApiError> {
    let failed = AtomicBool::new(false);
    let stop = || {
        should_stop(
            crate::signal::is_terminated(),
            failed.load(Ordering::SeqCst),
        )
    };
    let results = std::thread::scope(|scope| {
        let handles = servers
            .into_iter()
            .map(|server| {
                scope.spawn(|| {
                    let result = serve(server, config, token, &stop);
                    if result.is_err() {
                        failed.store(true, Ordering::SeqCst);
                    }
                    result
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|_| Err(ApiError::Recv("a server thread panicked".to_string())))
            })
            .collect::<Vec<_>>()
    });
    results.into_iter().collect()
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
    fn urls_loopback_only() {
        assert_eq!(
            listen_urls(addr("127.0.0.1:4000"), None, "t"),
            vec!["http://127.0.0.1:4000/?token=t".to_string()]
        );
    }

    #[test]
    fn urls_with_tailnet_ip_only() {
        let names = TailnetNames::default();
        assert_eq!(
            listen_urls(
                addr("127.0.0.1:4000"),
                Some((Ipv4Addr::new(100, 64, 0, 1), &names)),
                "t"
            ),
            vec![
                "http://127.0.0.1:4000/?token=t".to_string(),
                "http://100.64.0.1:4000/?token=t".to_string(),
            ]
        );
    }

    #[test]
    fn urls_with_fqdn_skip_short_name() {
        let names = TailnetNames {
            short: Some("box".to_string()),
            fqdn: Some("box.tail.ts.net".to_string()),
        };
        let urls = listen_urls(
            addr("127.0.0.1:4000"),
            Some((Ipv4Addr::new(100, 64, 0, 1), &names)),
            "t",
        );
        assert_eq!(urls.len(), 3);
        assert_eq!(urls[2], "http://box.tail.ts.net:4000/?token=t");
        assert!(urls.iter().all(|url| !url.contains("//box:")));
        assert!(urls.iter().all(|url| url.ends_with("?token=t")));
    }

    #[test]
    fn urls_short_name_alone_is_not_printed() {
        let names = TailnetNames {
            short: Some("box".to_string()),
            fqdn: None,
        };
        let urls = listen_urls(
            addr("127.0.0.1:4000"),
            Some((Ipv4Addr::new(100, 64, 0, 1), &names)),
            "t",
        );
        assert_eq!(urls.len(), 2);
    }

    #[test]
    fn should_stop_on_either_flag() {
        assert!(!should_stop(false, false));
        assert!(should_stop(true, false));
        assert!(should_stop(false, true));
        assert!(should_stop(true, true));
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

    #[test]
    fn ephemeral_retries_up_to_the_cap() {
        for attempt in 1..EPHEMERAL_BIND_ATTEMPTS {
            assert!(
                should_retry_bind(PortSource::Ephemeral, attempt),
                "{attempt}"
            );
        }
        assert!(!should_retry_bind(
            PortSource::Ephemeral,
            EPHEMERAL_BIND_ATTEMPTS
        ));
        assert!(!should_retry_bind(
            PortSource::Ephemeral,
            EPHEMERAL_BIND_ATTEMPTS + 1
        ));
    }

    #[test]
    fn fixed_ports_never_retry() {
        for source in [PortSource::Flag, PortSource::Config] {
            for attempt in 1..=EPHEMERAL_BIND_ATTEMPTS {
                assert!(!should_retry_bind(source, attempt), "{source:?} {attempt}");
            }
        }
    }
}
