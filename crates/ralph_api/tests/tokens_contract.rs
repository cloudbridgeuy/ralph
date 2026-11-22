#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]
#[allow(dead_code, reason = "each test crate uses a subset of the helpers")]
mod common;

use common::{assert_golden, Host, TestServer};

fn tokens_with(
    name: &str,
    method: &str,
    target: impl Fn(&str) -> String,
    host: Host,
    headers: impl Fn(&str) -> Vec<(String, String)>,
) {
    let server = TestServer::start();
    let token = server.token().to_string();
    let cookie = server.cookie_name();
    let owned: Vec<(String, String)> = headers(&token)
        .into_iter()
        .map(|(name, value)| (name, value.replace("@cookie@", &cookie)))
        .collect();
    let borrowed: Vec<(&str, &str)> = owned
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    let reply = server.exchange(method, &target(&token), host, &borrowed);
    assert_golden(name, &reply);
}

fn none(_: &str) -> Vec<(String, String)> {
    vec![]
}

#[test]
fn tokens_with_token() {
    tokens_with(
        "tokens_with_token",
        "GET",
        |t| format!("/tokens.css?token={t}"),
        Host::Own,
        none,
    );
}

#[test]
fn tokens_bearer() {
    tokens_with(
        "tokens_bearer",
        "GET",
        |_| "/tokens.css".into(),
        Host::Own,
        |t| vec![("Authorization".to_string(), format!("Bearer {t}"))],
    );
}

#[test]
fn tokens_cookie() {
    tokens_with(
        "tokens_cookie",
        "GET",
        |_| "/tokens.css".into(),
        Host::Own,
        |t| vec![("Cookie".to_string(), format!("@cookie@={t}"))],
    );
}

#[test]
fn tokens_without_token() {
    tokens_with(
        "tokens_without_token",
        "GET",
        |_| "/tokens.css".into(),
        Host::Own,
        none,
    );
}

#[test]
fn tokens_bad_host() {
    tokens_with(
        "tokens_bad_host",
        "GET",
        |t| format!("/tokens.css?token={t}"),
        Host::Named("evil.example"),
        none,
    );
}

#[test]
fn tokens_post() {
    tokens_with(
        "tokens_post",
        "POST",
        |t| format!("/tokens.css?token={t}"),
        Host::Own,
        none,
    );
}
