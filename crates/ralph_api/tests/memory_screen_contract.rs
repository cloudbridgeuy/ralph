#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]
#[allow(dead_code, reason = "each test crate uses a subset of the helpers")]
mod common;

use common::{Host, Reply, TestServer};

const CSP: &str = "default-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

const PRIMITIVES: [&str; 14] = [
    "r-box",
    "r-rule",
    "r-table",
    "r-kv",
    "r-tree",
    "r-pill",
    "r-status",
    "r-graph",
    "r-code",
    "r-prose",
    "r-spinner",
    "r-spark",
    "r-empty",
    "r-error",
];

fn get(server: &TestServer, target: &str, authed: bool) -> Reply {
    let cookie = format!("{}={}", server.cookie_name(), server.token());
    let headers: Vec<(&str, &str)> = if authed {
        vec![("Cookie", cookie.as_str())]
    } else {
        Vec::new()
    };
    server.exchange("GET", target, Host::Own, &headers)
}

fn header<'a>(reply: &'a Reply, name: &str) -> Option<&'a str> {
    reply
        .headers
        .iter()
        .find(|(found, _)| found.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn script() -> String {
    let server = TestServer::start();
    let reply = get(&server, "/app.js", true);
    assert_eq!(reply.status, 200);
    reply.body
}

#[test]
fn the_shell_loads_the_script_by_reference_only() {
    let server = TestServer::start();
    let reply = get(&server, "/", true);
    assert_eq!(reply.status, 200);
    assert!(reply
        .body
        .contains("<script src=\"/app.js\" defer></script>"));
    assert_eq!(reply.body.matches("<script").count(), 1);
    assert!(!reply.body.contains("style="));
    assert_eq!(header(&reply, "Content-Security-Policy"), Some(CSP));
}

#[test]
fn the_script_is_served_like_the_other_assets() {
    let server = TestServer::start();
    let reply = get(&server, "/app.js", true);
    assert_eq!(reply.status, 200);
    assert_eq!(
        header(&reply, "Content-Type"),
        Some("text/javascript; charset=utf-8")
    );
    assert_eq!(header(&reply, "Cache-Control"), Some("no-store"));
    assert_eq!(header(&reply, "X-Content-Type-Options"), Some("nosniff"));
    assert!(!reply.body.is_empty());
}

#[test]
fn the_script_needs_the_token() {
    let server = TestServer::start();
    assert_eq!(get(&server, "/app.js", false).status, 401);
}

#[test]
fn the_script_has_one_inner_html_and_no_unsafe_calls() {
    let body = script();
    assert_eq!(body.matches("innerHTML").count(), 1);
    for banned in [
        "eval(",
        "style.",
        "<img",
        "unsafe",
        "new Function",
        "document.write",
    ] {
        assert!(!body.contains(banned), "{banned}");
    }
}

#[test]
fn the_script_posts_markdown_only_to_the_render_route() {
    let body = script();
    assert!(body.contains("/api/render"));
    assert_eq!(body.matches("innerHTML").count(), 1);
    assert!(body.contains("credentials: \"same-origin\""));
    assert!(!body.contains("token"));
}

#[test]
fn every_r_tag_in_the_script_is_a_primitive() {
    let body = script();
    let mut found = 0;
    let mut rest = body.as_str();
    while let Some(at) = rest.find("r-") {
        let before = rest[..at].chars().last();
        let name: String = rest[at..]
            .chars()
            .take_while(|c| c.is_ascii_lowercase() || *c == '-')
            .collect();
        rest = &rest[at + 2..];
        if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            continue;
        }
        found += 1;
        assert!(PRIMITIVES.contains(&name.as_str()), "{name}");
    }
    assert!(found > 0);
}

#[test]
fn the_entries_table_opens_on_the_newest_page_unless_the_hash_sets_after() {
    let body = script();
    assert!(body.contains("function defaultAfter(total, size)"));
    assert!(body.contains("state.after === null ? defaultAfter("));
    assert!(body.contains("after: Number.isInteger(after) && after >= 0 ? after : null"));
}

#[test]
fn the_memory_ctx_carries_the_hash_after_never_the_resolved_one() {
    let body = script();
    assert!(body.contains("function memoryCtx(hashState)"));
    assert!(body.contains("const ctx = memoryCtx(hashState);"));
    assert!(!body.contains("const ctx = { thread: state.thread, after: state.after"));
    assert!(body.contains(
        "return { thread: hashState.thread, after: hashState.after, node: hashState.node };"
    ));
}

#[test]
fn an_explicit_after_past_the_end_is_clamped_before_the_entries_fetch() {
    let body = script();
    assert!(body.contains("function clampAfter(after, total, size)"));
    assert!(body.contains("clampAfter(state.after, roots.entries, PAGE_SIZE)"));
    let clamp = body.find("clampAfter(state.after").unwrap();
    let fetch = body.find("/api/memory/entries").unwrap();
    assert!(clamp < fetch);
}
