#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]
#[allow(dead_code, reason = "each test crate uses a subset of the helpers")]
mod common;

use common::{assert_golden, Host, Reply, TestServer};

const CSP: &str = "default-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

fn redacted(server: &TestServer, reply: Reply) -> Reply {
    let token = server.token().to_string();
    let cookie = server.cookie_name();
    reply.redacting(&[
        (token.as_str(), "<token>"),
        (cookie.as_str(), "ralph_token_<port>"),
    ])
}

fn get_with_cookie(server: &TestServer, target: &str) -> Reply {
    let cookie = format!("{}={}", server.cookie_name(), server.token());
    server.exchange("GET", target, Host::Own, &[("Cookie", cookie.as_str())])
}

#[test]
fn catalogue_with_cookie() {
    let server = TestServer::start();
    let reply = get_with_cookie(&server, "/catalogue");
    assert_golden("catalogue_with_cookie", &redacted(&server, reply));
}

#[test]
fn catalogue_without_token() {
    let server = TestServer::start();
    let reply = server.exchange("GET", "/catalogue", Host::Own, &[]);
    assert_golden("catalogue_without_token", &redacted(&server, reply));
}

#[test]
fn catalogue_bad_host() {
    let server = TestServer::start();
    let cookie = format!("{}={}", server.cookie_name(), server.token());
    let reply = server.exchange(
        "GET",
        "/catalogue",
        Host::Named("evil.example"),
        &[("Cookie", cookie.as_str())],
    );
    assert_golden("catalogue_bad_host", &redacted(&server, reply));
}

#[test]
fn catalogue_post() {
    let server = TestServer::start();
    let cookie = format!("{}={}", server.cookie_name(), server.token());
    let reply = server.exchange(
        "POST",
        "/catalogue",
        Host::Own,
        &[("Cookie", cookie.as_str())],
    );
    assert_golden("catalogue_post", &redacted(&server, reply));
}

#[test]
fn catalogue_policy_matches_the_shell() {
    let server = TestServer::start();
    let page = get_with_cookie(&server, "/catalogue");
    let shell = get_with_cookie(&server, "/");
    let policy = |reply: &Reply| {
        reply
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("Content-Security-Policy"))
            .map(|(_, value)| value.clone())
    };
    assert_eq!(policy(&page).as_deref(), Some(CSP));
    assert_eq!(policy(&page), policy(&shell));
}

#[test]
fn the_shell_nav_reaches_the_catalogue() {
    let server = TestServer::start();
    let shell = get_with_cookie(&server, "/");
    assert!(shell.body.contains(
        "<a href=\"/catalogue\"><span class=\"n\" aria-hidden=\"true\">6</span>catalogue</a>"
    ));
}
