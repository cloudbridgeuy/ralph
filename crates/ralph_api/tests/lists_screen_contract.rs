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

fn block(js: &str, start: &str) -> String {
    let from = js.find(start).unwrap();
    let rest = &js[from..];
    let end = rest[1..]
        .find("\nfunction ")
        .map_or(rest.len(), |at| at + 1);
    rest[..end].to_string()
}

#[test]
fn the_nav_lists_the_screens_in_order_and_the_catalogue_last() {
    let html = fetch("/");
    let links = [
        "<a href=\"#/memory\"><span class=\"n\" aria-hidden=\"true\">1</span>memory</a>",
        "<a href=\"#/threads\"><span class=\"n\" aria-hidden=\"true\">2</span>threads</a>",
        "<a href=\"#/workers\"><span class=\"n\" aria-hidden=\"true\">3</span>workers</a>",
        "<a href=\"#/schedules\"><span class=\"n\" aria-hidden=\"true\">4</span>schedules</a>",
        "<a href=\"#/watches\"><span class=\"n\" aria-hidden=\"true\">5</span>watches</a>",
        "<a href=\"#/config\"><span class=\"n\" aria-hidden=\"true\">6</span>config</a>",
        "<a href=\"/catalogue\"><span class=\"n\" aria-hidden=\"true\">7</span>catalogue</a>",
    ];
    let at: Vec<usize> = links.iter().map(|link| html.find(link).unwrap()).collect();
    assert!(at.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn the_script_lists_the_three_screens_after_threads() {
    let js = fetch("/app.js");
    assert!(js.contains(
        "const SCREENS = [\"/memory\", \"/threads\", \"/workers\", \"/schedules\", \"/watches\", \"/config\"];"
    ));
}

#[test]
fn each_screen_loads_its_route_without_parameters() {
    let js = fetch("/app.js");
    for path in ["/api/workers", "/api/schedules", "/api/watches"] {
        assert!(js.contains(&format!("\"{path}\"")), "{path}");
    }
    assert!(js.contains("function loadList"));
    assert!(js.contains("api(spec.path)"));
}

#[test]
fn the_tables_have_the_agreed_columns() {
    let js = fetch("/app.js");
    for columns in [
        "[\"Worker\", \"Harness\", \"State\", \"Pending\", \"Ticket\", \"Last report\"]",
        "[\"Task\", \"Kind\", \"Status\", \"Runs at\", \"Repeat\", \"Late\"]",
        "[\"Watch\", \"Kind\", \"Started\", \"Wakes\", \"Expires\"]",
    ] {
        assert!(js.contains(columns), "{columns}");
    }
}

#[test]
fn rows_render_in_an_r_table_with_pills() {
    let drawn = block(&fetch("/app.js"), "function listTable");
    assert!(drawn.contains("el(\"r-table\""));
    assert!(drawn.contains("el(\"th\""));
    let pill = block(&fetch("/app.js"), "function listPill");
    assert!(pill.contains("el(\"r-pill\""));
}

#[test]
fn workers_use_the_server_tone_and_schedules_map_the_status() {
    let js = fetch("/app.js");
    assert!(js.contains("listPill(row.state, row.tone)"));
    assert!(js.contains("pending: \"info\""));
    assert!(js.contains("running: \"ok\""));
    assert!(js.contains("proposed: \"neutral\""));
    assert!(js.contains("row.late ? listPill(\"late\", \"bad\")"));
    assert!(js.contains("persistent: \"accent\""));
    assert!(js.contains("normal: \"neutral\""));
}

#[test]
fn an_empty_list_is_an_empty_status() {
    let js = fetch("/app.js");
    for what in ["No live workers", "No scheduled tasks", "No active watches"] {
        assert!(
            js.contains(&format!("emptyState(\"{what}\")")) || js.contains(&format!("\"{what}\"")),
            "{what}"
        );
    }
    let drawn = block(&js, "function drawList");
    assert!(drawn.contains("emptyState(spec.empty)"));
}

#[test]
fn a_failure_is_an_alert_with_the_name_the_why_and_a_retry_button() {
    let js = fetch("/app.js");
    let failed = block(&js, "function listFailure");
    assert!(failed.contains("role: \"alert\""));
    assert!(failed.contains(" did not load"));
    assert!(failed.contains("error.why"));
    assert!(failed.contains("class: \"btn\""));
    for name in ["Workers", "Schedules", "Watches"] {
        assert!(js.contains(&format!("name: \"{name}\"")), "{name}");
    }
}

#[test]
fn the_list_screens_do_not_poll() {
    let js = fetch("/app.js");
    assert!(!js.contains("setInterval"));
}

#[test]
fn the_router_picks_the_list_screens_by_route() {
    let shown = block(&fetch("/app.js"), "async function show(");
    assert!(shown.contains("LISTS[state.route]"));
    assert!(shown.contains("drawList(main, list, data)"));
    assert!(shown.contains("listFailure(list.name, error"));
}
