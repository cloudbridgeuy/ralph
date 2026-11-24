#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]
#[allow(dead_code, reason = "each test crate uses a subset of the helpers")]
mod common;

use common::{assert_golden, Host, TestServer};

fn golden_case(name: &str, method: &str, target: &str, headers: &[(&str, String)]) {
    let server = TestServer::start();
    let token = server.token().to_string();
    let target = target.replace("{t}", &token);
    let owned: Vec<(&str, String)> = headers
        .iter()
        .map(|(name, value)| {
            (
                *name,
                value
                    .replace("{t}", &token)
                    .replace("{c}", &server.cookie_name()),
            )
        })
        .collect();
    let borrowed: Vec<(&str, &str)> = owned.iter().map(|(n, v)| (*n, v.as_str())).collect();
    let reply = server.exchange(method, &target, Host::Own, &borrowed);
    let cookie = server.cookie_name();
    let swaps = [
        (token.as_str(), "<token>"),
        (cookie.as_str(), "ralph_token_<port>"),
    ];
    assert_golden(name, &reply.redacting(&swaps));
}

fn cookie() -> Vec<(&'static str, String)> {
    vec![("Cookie", "{c}={t}".to_string())]
}

#[test]
fn shell_token_redirect() {
    golden_case("shell_token_redirect", "GET", "/?token={t}", &[]);
}

#[test]
fn shell_with_cookie() {
    golden_case("shell_with_cookie", "GET", "/", &cookie());
}

#[test]
fn shell_without_token() {
    golden_case("shell_without_token", "GET", "/", &[]);
}

#[test]
fn shell_post() {
    golden_case("shell_post", "POST", "/?token={t}", &[]);
}

#[test]
fn shell_wrong_port_cookie() {
    let headers = vec![("Cookie", "ralph_token_1={t}".to_string())];
    golden_case("shell_wrong_port_cookie", "GET", "/", &headers);
}

#[test]
fn css_with_cookie() {
    golden_case("css_with_cookie", "GET", "/assets/app.css", &cookie());
}

#[test]
fn asset_unknown() {
    golden_case("asset_unknown", "GET", "/assets/nope.css", &cookie());
}

#[test]
fn font_with_cookie() {
    let server = TestServer::start();
    let cookie = format!("{}={}", server.cookie_name(), server.token());
    let reply = server.exchange(
        "GET",
        "/assets/geist-mono.woff2",
        Host::Own,
        &[("Cookie", &cookie)],
    );
    assert_eq!(reply.status, 200);
    let header = |name: &str| {
        reply
            .headers
            .iter()
            .find(|(found, _)| found.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    };
    assert_eq!(header("Content-Type"), Some("font/woff2"));
    assert_eq!(header("Cache-Control"), Some("no-store"));
    assert_eq!(header("X-Content-Type-Options"), Some("nosniff"));
    assert_eq!(reply.body_bytes.len(), 71_212);
    assert!(reply.body_bytes.starts_with(b"wOF2"));
}

#[test]
fn css_post() {
    golden_case("asset_post_css", "POST", "/assets/app.css", &cookie());
}

#[test]
fn script_post() {
    golden_case("asset_post_js", "POST", "/app.js", &cookie());
}

#[test]
fn font_post() {
    golden_case(
        "asset_post_font",
        "POST",
        "/assets/geist-mono.woff2",
        &cookie(),
    );
}
