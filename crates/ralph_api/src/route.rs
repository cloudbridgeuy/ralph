use crate::assets::{self, Asset, SHELL_PATH};
use crate::catalogue;
use crate::config_api;
use crate::memory_api;
use crate::render_api::{reject_oversize, render};
use crate::token::{extract_tokens, Token};
use std::path::PathBuf;

const HEALTH_PATH: &str = "/api/health";
const TOKENS_PATH: &str = "/tokens.css";
const RENDER_PATH: &str = "/api/render";
const CONFIG_PATH: &str = "/api/config";
const MEMORY_PATH: &str = "/api/memory";
const ENTRIES_PATH: &str = "/api/memory/entries";
const THREADS_PATH: &str = "/api/memory/threads";
const CATALOGUE_PATH: &str = "/catalogue";
const CATALOGUE_FIXTURE: &str = include_str!("../fixtures/catalogue.json");
const HTML: &str = "text/html; charset=utf-8";
const JSON: &str = "application/json";
const CSS: &str = "text/css; charset=utf-8";
const PLAIN: &str = "text/plain; charset=utf-8";
const NO_STORE: (&str, &str) = ("Cache-Control", "no-store");
const NO_REFERRER: (&str, &str) = ("Referrer-Policy", "no-referrer");
const NO_SNIFF: (&str, &str) = ("X-Content-Type-Options", "nosniff");
const SHELL_POLICY: (&str, &str) = (
    "Content-Security-Policy",
    "default-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApiRequest {
    pub method: String,
    pub path: String,
    pub query: String,
    pub host: Option<String>,
    pub authorization: Option<String>,
    pub cookies: Vec<String>,
    pub body: Vec<u8>,
    pub content_type: Option<String>,
    pub body_too_large: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApiResponse {
    pub status: u16,
    pub content_type: &'static str,
    pub headers: Vec<(&'static str, &'static str)>,
    pub body: Vec<u8>,
    pub set_cookie: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ApiConfig {
    pub store: PathBuf,
    pub allowed_hosts: Vec<String>,
    pub version: String,
    pub tokens_css: String,
    pub cookie_name: String,
    pub global_config: PathBuf,
    pub env: fn(&str) -> Option<String>,
}

pub(crate) fn route(request: &ApiRequest, config: &ApiConfig, token: &Token) -> ApiResponse {
    if let Some(rejected) = reject_oversize(request) {
        return rejected;
    }
    if !host_allowed(request.host.as_deref(), &config.allowed_hosts) {
        return error(403, "forbidden host");
    }
    if !authorized(request, token, &config.cookie_name) {
        return error(401, "unauthorized");
    }
    match (request.path.as_str(), request.method.as_str()) {
        (HEALTH_PATH, "GET") => health(&config.version),
        (HEALTH_PATH, _) => with_header(error(405, "method not allowed"), ("Allow", "GET")),
        (TOKENS_PATH, "GET") => css_response(&config.tokens_css),
        (TOKENS_PATH, _) => with_header(error(405, "method not allowed"), ("Allow", "GET")),
        (RENDER_PATH, "POST") => render(request),
        (RENDER_PATH, _) => with_header(error(405, "method not allowed"), ("Allow", "POST")),
        (CATALOGUE_PATH, "GET") => catalogue_response(),
        (CATALOGUE_PATH, _) => with_header(error(405, "method not allowed"), ("Allow", "GET")),
        (MEMORY_PATH, "GET") => memory_api::memory(request, config),
        (ENTRIES_PATH, "GET") => memory_api::entries(request, config),
        (THREADS_PATH, "GET") => memory_api::threads(request, config),
        (CONFIG_PATH, "GET") => config_api::config(request, config),
        (CONFIG_PATH, _) => with_header(error(405, "method not allowed"), ("Allow", "GET")),
        (MEMORY_PATH | ENTRIES_PATH | THREADS_PATH, _) => {
            with_header(error(405, "method not allowed"), ("Allow", "GET"))
        }
        _ => match assets::find(&request.path) {
            Some(asset) => asset_response(asset, request, config, token),
            None => error(404, "not found"),
        },
    }
}

pub(crate) fn accepts_body(request: &ApiRequest, config: &ApiConfig, token: &Token) -> bool {
    request.method == "POST"
        && request.path == RENDER_PATH
        && host_allowed(request.host.as_deref(), &config.allowed_hosts)
        && authorized(request, token, &config.cookie_name)
}

fn catalogue_response() -> ApiResponse {
    ApiResponse {
        status: 200,
        content_type: HTML,
        headers: vec![NO_STORE, NO_REFERRER, NO_SNIFF, SHELL_POLICY],
        body: catalogue::page(CATALOGUE_FIXTURE).into_bytes(),
        set_cookie: None,
    }
}

fn asset_response(
    asset: &Asset,
    request: &ApiRequest,
    config: &ApiConfig,
    token: &Token,
) -> ApiResponse {
    if request.method != "GET" {
        return with_header(error(405, "method not allowed"), ("Allow", "GET"));
    }
    if asset.path == SHELL_PATH && query_authorized(request, token, &config.cookie_name) {
        return hand_off(token, &config.cookie_name);
    }
    let mut headers = vec![NO_STORE, NO_REFERRER, NO_SNIFF];
    if asset.path == SHELL_PATH {
        headers.push(SHELL_POLICY);
    }
    ApiResponse {
        status: 200,
        content_type: asset.content_type,
        headers,
        body: asset.bytes.to_vec(),
        set_cookie: None,
    }
}

fn hand_off(token: &Token, cookie_name: &str) -> ApiResponse {
    ApiResponse {
        status: 302,
        content_type: PLAIN,
        headers: vec![NO_STORE, NO_REFERRER, ("Location", SHELL_PATH)],
        body: Vec::new(),
        set_cookie: Some(format!(
            "{cookie_name}={}; Path=/; HttpOnly; SameSite=Strict",
            token.as_str()
        )),
    }
}

fn query_authorized(request: &ApiRequest, token: &Token, cookie_name: &str) -> bool {
    extract_tokens(None, &request.query, &[], cookie_name)
        .iter()
        .any(|candidate| token.matches(candidate))
}

pub(crate) fn overloaded() -> ApiResponse {
    error(503, "busy")
}

fn host_allowed(host: Option<&str>, allowed: &[String]) -> bool {
    host.is_some_and(|host| allowed.iter().any(|name| name.eq_ignore_ascii_case(host)))
}

fn authorized(request: &ApiRequest, token: &Token, cookie_name: &str) -> bool {
    let cookies: Vec<&str> = request.cookies.iter().map(String::as_str).collect();
    extract_tokens(
        request.authorization.as_deref(),
        &request.query,
        &cookies,
        cookie_name,
    )
    .iter()
    .any(|candidate| token.matches(candidate))
}

fn health(version: &str) -> ApiResponse {
    json_response(200, serde_json::json!({"status": "ok", "version": version}))
}

fn css_response(body: &str) -> ApiResponse {
    ApiResponse {
        status: 200,
        content_type: CSS,
        headers: vec![NO_STORE],
        body: body.to_string().into_bytes(),
        set_cookie: None,
    }
}

pub(crate) fn error(status: u16, message: &str) -> ApiResponse {
    json_response(status, serde_json::json!({ "error": message }))
}

pub(crate) fn json_ok(body: serde_json::Value) -> ApiResponse {
    ApiResponse {
        status: 200,
        content_type: JSON,
        headers: vec![NO_STORE, NO_REFERRER, NO_SNIFF],
        body: body.to_string().into_bytes(),
        set_cookie: None,
    }
}

pub(crate) fn json_response(status: u16, body: serde_json::Value) -> ApiResponse {
    ApiResponse {
        status,
        content_type: JSON,
        headers: vec![NO_STORE],
        body: body.to_string().into_bytes(),
        set_cookie: None,
    }
}

fn with_header(mut response: ApiResponse, header: (&'static str, &'static str)) -> ApiResponse {
    response.headers.push(header);
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    const TOKEN: &str = "t0k3n";

    fn token() -> Token {
        Token::from_value(TOKEN)
    }

    fn config() -> ApiConfig {
        ApiConfig {
            store: PathBuf::from("/secret/store"),
            allowed_hosts: vec!["h.test:1".to_string(), "alias.test:1".to_string()],
            version: "9.9.9".to_string(),
            tokens_css: ":root {\n  --r-font: x;\n}\n".to_string(),
            cookie_name: "ralph_token_1".to_string(),
            global_config: PathBuf::from("/secret/global/config.toml"),
            env: |_| None,
        }
    }

    fn request(method: &str, path: &str, query: &str) -> ApiRequest {
        ApiRequest {
            method: method.to_string(),
            path: path.to_string(),
            query: query.to_string(),
            host: Some("h.test:1".to_string()),
            authorization: None,
            cookies: vec![],
            body: vec![],
            content_type: None,
            body_too_large: false,
        }
    }

    fn authed(method: &str, path: &str) -> ApiRequest {
        request(method, path, &format!("token={TOKEN}"))
    }

    fn body(response: &ApiResponse) -> Value {
        serde_json::from_slice(&response.body).unwrap()
    }

    #[test]
    fn health_with_token_is_200_json_ok() {
        let response = route(&authed("GET", "/api/health"), &config(), &token());
        assert_eq!(response.status, 200);
        assert_eq!(body(&response), json!({"status": "ok", "version": "9.9.9"}));
        assert!(!String::from_utf8_lossy(&response.body).contains("/secret/store"));
    }

    #[test]
    fn oversize_declared_body_with_auth_is_413() {
        let mut req = authed("POST", "/api/health");
        req.body_too_large = true;
        let response = route(&req, &config(), &token());
        assert_eq!(response.status, 413);
        assert_eq!(body(&response), json!({"error": "too large"}));
    }

    #[test]
    fn oversize_body_without_token_is_413() {
        let mut req = request("POST", "/api/health", "");
        req.body_too_large = true;
        let response = route(&req, &config(), &token());
        assert_eq!(response.status, 413);
        assert_eq!(body(&response), json!({"error": "too large"}));
    }

    #[test]
    fn oversize_body_on_bad_host_is_413() {
        let mut req = authed("POST", "/api/health");
        req.host = Some("evil.test".to_string());
        req.body_too_large = true;
        let response = route(&req, &config(), &token());
        assert_eq!(response.status, 413);
        assert_eq!(body(&response), json!({"error": "too large"}));
    }

    #[test]
    fn oversize_body_on_unknown_path_is_413() {
        let mut req = request("PUT", "/nope", "");
        req.body_too_large = true;
        assert_eq!(route(&req, &config(), &token()).status, 413);
    }

    #[test]
    fn oversize_authorised_render_is_413() {
        let mut req = authed("POST", "/api/render");
        req.body_too_large = true;
        assert_eq!(route(&req, &config(), &token()).status, 413);
    }

    #[test]
    fn body_at_the_cap_without_token_is_still_401() {
        let mut req = request("POST", "/api/health", "");
        req.body = vec![0; crate::render_api::MAX_BODY];
        assert_eq!(route(&req, &config(), &token()).status, 401);
    }

    #[test]
    fn only_an_authorised_render_post_accepts_a_body() {
        let (config, token) = (config(), token());
        assert!(accepts_body(
            &authed("POST", "/api/render"),
            &config,
            &token
        ));
        assert!(!accepts_body(
            &authed("GET", "/api/render"),
            &config,
            &token
        ));
        assert!(!accepts_body(
            &authed("POST", "/api/health"),
            &config,
            &token
        ));
        assert!(!accepts_body(&authed("POST", "/x"), &config, &token));
        let unauthorised = request("POST", "/api/render", "");
        assert!(!accepts_body(&unauthorised, &config, &token));
        let mut bad_host = authed("POST", "/api/render");
        bad_host.host = Some("evil.test".to_string());
        assert!(!accepts_body(&bad_host, &config, &token));
        bad_host.host = None;
        assert!(!accepts_body(&bad_host, &config, &token));
    }

    #[test]
    fn health_without_token_is_401() {
        let response = route(&request("GET", "/api/health", ""), &config(), &token());
        assert_eq!(response.status, 401);
        assert_eq!(body(&response), json!({"error": "unauthorized"}));
    }

    #[test]
    fn health_with_wrong_token_is_401() {
        let response = route(
            &request("GET", "/api/health", "token=nope"),
            &config(),
            &token(),
        );
        assert_eq!(response.status, 401);
    }

    #[test]
    fn unknown_path_with_token_is_404() {
        let response = route(&authed("GET", "/api/other"), &config(), &token());
        assert_eq!(response.status, 404);
        assert_eq!(body(&response), json!({"error": "not found"}));
    }

    #[test]
    fn unknown_path_without_token_is_401() {
        let response = route(&request("GET", "/api/other", ""), &config(), &token());
        assert_eq!(response.status, 401);
    }

    #[test]
    fn post_health_is_405_and_allow_get() {
        let response = route(&authed("POST", "/api/health"), &config(), &token());
        assert_eq!(response.status, 405);
        assert!(response.headers.contains(&("Allow", "GET")));
    }

    #[test]
    fn bad_host_is_403() {
        let mut bad = authed("GET", "/api/health");
        bad.host = Some("evil.example".to_string());
        let response = route(&bad, &config(), &token());
        assert_eq!(response.status, 403);
        assert_eq!(body(&response), json!({"error": "forbidden host"}));
    }

    #[test]
    fn missing_host_is_403() {
        let mut bad = authed("GET", "/api/health");
        bad.host = None;
        assert_eq!(route(&bad, &config(), &token()).status, 403);
    }

    #[test]
    fn allowed_hosts_come_from_config() {
        let mut other = authed("GET", "/api/health");
        other.host = Some("alias.test:1".to_string());
        assert_eq!(route(&other, &config(), &token()).status, 200);
        let mut empty = config();
        empty.allowed_hosts.clear();
        assert_eq!(
            route(&authed("GET", "/api/health"), &empty, &token()).status,
            403
        );
    }

    #[test]
    fn tokens_with_token_is_200_css_no_store() {
        let response = route(&authed("GET", "/tokens.css"), &config(), &token());
        assert_eq!(response.status, 200);
        assert_eq!(response.content_type, "text/css; charset=utf-8");
        assert!(response.headers.contains(&("Cache-Control", "no-store")));
    }

    #[test]
    fn tokens_body_is_the_configured_css() {
        let response = route(&authed("GET", "/tokens.css"), &config(), &token());
        assert_eq!(response.body, config().tokens_css.as_bytes());
    }

    #[test]
    fn tokens_without_token_is_401() {
        let response = route(&request("GET", "/tokens.css", ""), &config(), &token());
        assert_eq!(response.status, 401);
        assert_eq!(body(&response), json!({"error": "unauthorized"}));
    }

    #[test]
    fn tokens_bad_host_is_403() {
        let mut bad = authed("GET", "/tokens.css");
        bad.host = Some("evil.example".to_string());
        assert_eq!(route(&bad, &config(), &token()).status, 403);
    }

    #[test]
    fn post_tokens_is_405_and_allow_get() {
        let response = route(&authed("POST", "/tokens.css"), &config(), &token());
        assert_eq!(response.status, 405);
        assert!(response.headers.contains(&("Allow", "GET")));
        assert_eq!(response.content_type, "application/json");
    }

    #[test]
    fn overloaded_is_503_json_no_store() {
        let response = overloaded();
        assert_eq!(response.status, 503);
        assert_eq!(body(&response), json!({"error": "busy"}));
        assert!(response.headers.contains(&("Cache-Control", "no-store")));
    }

    #[test]
    fn every_response_is_json_no_store() {
        let mut bad_host = authed("GET", "/api/health");
        bad_host.host = None;
        let responses = [
            route(&authed("GET", "/api/health"), &config(), &token()),
            route(&request("GET", "/api/health", ""), &config(), &token()),
            route(&authed("GET", "/nope"), &config(), &token()),
            route(&authed("POST", "/api/health"), &config(), &token()),
            route(&bad_host, &config(), &token()),
        ];
        for response in responses {
            assert_eq!(response.content_type, "application/json");
            assert!(response.headers.contains(&("Cache-Control", "no-store")));
        }
    }

    const CSP: &str = "default-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";
    const COOKIE: &str = "ralph_token_1=t0k3n; Path=/; HttpOnly; SameSite=Strict";

    fn with_cookie(path: &str, cookie: &str) -> ApiRequest {
        let mut req = request("GET", path, "");
        req.cookies = vec![cookie.to_string()];
        req
    }

    fn bytes(response: &ApiResponse) -> &[u8] {
        response.body.as_ref()
    }

    #[test]
    fn shell_without_token_is_401() {
        let response = route(&request("GET", "/", ""), &config(), &token());
        assert_eq!(response.status, 401);
    }

    #[test]
    fn shell_with_query_token_redirects_and_sets_the_cookie() {
        let response = route(&authed("GET", "/"), &config(), &token());
        assert_eq!(response.status, 302);
        assert!(response.headers.contains(&("Location", "/")));
        assert!(response.headers.contains(&("Cache-Control", "no-store")));
        assert_eq!(response.set_cookie.as_deref(), Some(COOKIE));
        assert!(response.body.is_empty());
    }

    #[test]
    fn shell_with_the_cookie_is_html_with_a_policy() {
        let response = route(
            &with_cookie("/", "ralph_token_1=t0k3n"),
            &config(),
            &token(),
        );
        assert_eq!(response.status, 200);
        assert_eq!(response.content_type, "text/html; charset=utf-8");
        for header in [
            ("Cache-Control", "no-store"),
            ("Content-Security-Policy", CSP),
            ("Referrer-Policy", "no-referrer"),
            ("X-Content-Type-Options", "nosniff"),
        ] {
            assert!(response.headers.contains(&header), "{header:?}");
        }
        assert!(bytes(&response).starts_with(b"<!doctype html>"));
        assert_eq!(response.set_cookie, None);
    }

    #[test]
    fn a_cookie_for_another_port_is_not_accepted() {
        let own = route(
            &with_cookie("/", "ralph_token_1=t0k3n"),
            &config(),
            &token(),
        );
        assert_eq!(own.status, 200);
        for name in ["ralph_token_2", "ralph_token", "ralph_token_11"] {
            let other = route(
                &with_cookie("/", &format!("{name}=t0k3n")),
                &config(),
                &token(),
            );
            assert_eq!(other.status, 401, "{name}");
        }
    }

    #[test]
    fn assets_are_served_with_their_types() {
        for (path, kind) in [
            ("/assets/app.css", "text/css; charset=utf-8"),
            ("/assets/geist-mono.woff2", "font/woff2"),
        ] {
            let response = route(
                &with_cookie(path, "ralph_token_1=t0k3n"),
                &config(),
                &token(),
            );
            assert_eq!(response.status, 200, "{path}");
            assert_eq!(response.content_type, kind);
            assert!(response.headers.contains(&("Cache-Control", "no-store")));
            assert!(response
                .headers
                .contains(&("X-Content-Type-Options", "nosniff")));
            assert!(!bytes(&response).is_empty());
        }
        let font = route(
            &authed("GET", "/assets/geist-mono.woff2"),
            &config(),
            &token(),
        );
        assert!(bytes(&font).starts_with(b"wOF2"));
    }

    #[test]
    fn post_to_the_shell_or_an_asset_is_405_and_allow_get() {
        for path in ["/", "/assets/app.css", "/assets/geist-mono.woff2"] {
            let response = route(&authed("POST", path), &config(), &token());
            assert_eq!(response.status, 405, "{path}");
            assert!(response.headers.contains(&("Allow", "GET")));
        }
    }

    #[test]
    fn unknown_assets_are_404_and_bad_hosts_are_403() {
        for path in [
            "/assets/nope.css",
            "/assets/",
            "/assets/../tokens.css",
            "/index.html",
        ] {
            assert_eq!(
                route(&authed("GET", path), &config(), &token()).status,
                404,
                "{path}"
            );
        }
        let mut bad = authed("GET", "/assets/app.css");
        bad.host = Some("evil.example".to_string());
        assert_eq!(route(&bad, &config(), &token()).status, 403);
    }

    #[test]
    fn catalogue_with_the_cookie_is_html_with_the_shell_policy() {
        let response = route(
            &with_cookie("/catalogue", "ralph_token_1=t0k3n"),
            &config(),
            &token(),
        );
        assert_eq!(response.status, 200);
        assert_eq!(response.content_type, "text/html; charset=utf-8");
        for header in [
            ("Cache-Control", "no-store"),
            ("Content-Security-Policy", CSP),
            ("Referrer-Policy", "no-referrer"),
            ("X-Content-Type-Options", "nosniff"),
        ] {
            assert!(response.headers.contains(&header), "{header:?}");
        }
        assert!(bytes(&response).starts_with(b"<!doctype html>"));
        assert_eq!(response.set_cookie, None);
    }

    #[test]
    fn catalogue_without_token_is_401() {
        let response = route(&request("GET", "/catalogue", ""), &config(), &token());
        assert_eq!(response.status, 401);
    }

    #[test]
    fn catalogue_bad_host_is_403() {
        let mut bad = authed("GET", "/catalogue");
        bad.host = Some("evil.example".to_string());
        assert_eq!(route(&bad, &config(), &token()).status, 403);
    }

    #[test]
    fn post_catalogue_is_405_and_allow_get() {
        let response = route(&authed("POST", "/catalogue"), &config(), &token());
        assert_eq!(response.status, 405);
        assert!(response.headers.contains(&("Allow", "GET")));
    }

    #[test]
    fn host_match_ignores_ascii_case_but_nothing_else() {
        let allowed = vec!["phone.tail1234.ts.net:8080".to_string()];
        assert!(host_allowed(Some("Phone.TAIL1234.ts.net:8080"), &allowed));
        for bad in [
            "evil.example:8080",
            "phone.tail1234.ts.net:8081",
            "phone.tail1234.ts.net",
            "phone.tail1234.ts.net.:8080",
            "user@phone.tail1234.ts.net:8080",
            "",
        ] {
            assert!(!host_allowed(Some(bad), &allowed), "{bad}");
        }
        assert!(!host_allowed(None, &allowed));
    }
}
