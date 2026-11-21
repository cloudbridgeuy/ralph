use crate::chat::session::store_dir;
use crate::cli::WebArgs;
use crate::web_tokens::render_tokens;
use ralph_api::{allowed_hosts, bind, bind_hosts, serve, ApiConfig, BindPolicy, Token};
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
    let server = bind(SocketAddr::new(ip, args.port), &policy)?;
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

fn api_config(store: PathBuf, bound: SocketAddr, version: &str, tokens_css: String) -> ApiConfig {
    ApiConfig {
        store,
        allowed_hosts: allowed_hosts(&bind_hosts(bound.ip()), bound.port()),
        version: version.to_string(),
        tokens_css,
    }
}

fn listen_url(bound: SocketAddr, token: &str) -> String {
    format!("http://{bound}/api/health?token={token}")
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
    fn listen_url_points_at_health_with_the_token() {
        assert_eq!(
            listen_url(addr("127.0.0.1:4000"), "abc"),
            "http://127.0.0.1:4000/api/health?token=abc"
        );
    }

    #[test]
    fn listen_url_brackets_ipv6() {
        assert_eq!(
            listen_url(addr("[::1]:4000"), "abc"),
            "http://[::1]:4000/api/health?token=abc"
        );
    }
}
