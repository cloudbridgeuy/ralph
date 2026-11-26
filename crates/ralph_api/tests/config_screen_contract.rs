#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]
#[allow(dead_code, reason = "each test crate uses a subset of the helpers")]
mod common;

use common::{Host, TestServer};

fn fetch(path: &str) -> String {
    let server = TestServer::start();
    let cookie = format!("{}={}", server.cookie_name(), server.token());
    let reply = server.exchange("GET", path, Host::Own, &[("Cookie", cookie.as_str())]);
    assert_eq!(reply.status, 200);
    reply.body
}

fn config_block(js: &str) -> String {
    let start = js.find("function drawConfig").unwrap();
    let rest = &js[start..];
    let end = rest[1..]
        .find("\nfunction ")
        .map_or(rest.len(), |at| at + 1);
    rest[..end].to_string()
}

#[test]
fn the_nav_puts_config_third_and_the_catalogue_fourth() {
    let html = fetch("/");
    let memory = html.find("<a href=\"#/memory\">").unwrap();
    let config = html
        .find("<a href=\"#/config\"><span class=\"n\" aria-hidden=\"true\">3</span>config</a>")
        .unwrap();
    let catalogue = html
        .find("<a href=\"/catalogue\"><span class=\"n\" aria-hidden=\"true\">4</span>catalogue</a>")
        .unwrap();
    assert!(memory < config && config < catalogue);
}

#[test]
fn the_script_lists_config_as_a_screen_after_threads() {
    let js = fetch("/app.js");
    assert!(js.contains("const SCREENS = [\"/memory\", \"/threads\", \"/config\"];"));
}

#[test]
fn the_script_loads_the_config_route_without_parameters() {
    let js = fetch("/app.js");
    assert!(js.contains("function loadConfig"));
    assert!(js.contains("api(\"/api/config\")"));
}

#[test]
fn each_section_is_a_box_titled_in_the_border_with_its_row_count_aside() {
    let block = config_block(&fetch("/app.js"));
    assert!(block.contains("el(\"r-box\""));
    assert!(block.contains("class: \"title\""));
    assert!(block.contains("class: \"aside\""));
    assert!(block.contains("row.section"));
}

#[test]
fn rows_are_kv_lines_with_the_source_as_a_pill() {
    let block = config_block(&fetch("/app.js"));
    assert!(block.contains("el(\"r-kv\""));
    assert!(block.contains("el(\"r-pill\""));
    assert!(block.contains("row.source"));
    assert!(block.contains("row.source === \"default\""));
}

#[test]
fn a_redacted_row_shows_a_muted_hidden_mark_not_the_value() {
    let block = config_block(&fetch("/app.js"));
    assert!(block.contains("row.redacted"));
    assert!(block.contains("\"hidden\""));
}

#[test]
fn no_rows_is_an_empty_status() {
    let block = config_block(&fetch("/app.js"));
    assert!(block.contains("emptyState(\"No settings\")"));
}

#[test]
fn a_failure_is_an_alert_with_what_why_next_and_a_retry_button() {
    let js = fetch("/app.js");
    assert!(js.contains("Config did not load"));
    assert!(js.contains("Fix config.toml, then retry"));
    assert!(js.contains("error.why"));
    assert!(js.contains("role: \"alert\""));
    assert!(js.contains("class: \"btn\""));
}
