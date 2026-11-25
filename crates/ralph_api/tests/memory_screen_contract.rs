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

#[test]
fn append_results_are_never_taken_as_the_new_child() {
    let body = script();
    for name in ["row", "summary", "list", "option"] {
        let bound = format!("const {name} = append(");
        assert!(
            !body.contains(&bound),
            "{bound} binds the parent, not the child"
        );
    }
    assert!(!body.contains("append(group, rowItem()).appendChild("));
    assert!(body.contains("function zoomRows(zoom)"));
}

const TREE_EXTENT: &str = include_str!("golden/render/tree_extent.html");
const TREE_PENDING: &str = include_str!("golden/render/tree_pending.html");

fn class_names(html: &str) -> Vec<String> {
    let mut found: Vec<String> = html
        .split("class=\"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .flat_map(|names| names.split(' ').map(str::to_string))
        .collect();
    found.sort();
    found.dedup();
    found
}

#[test]
fn the_script_builds_every_class_the_rust_tree_golden_emits() {
    let body = script();
    let mut names = class_names(TREE_EXTENT);
    names.extend(class_names(TREE_PENDING));
    for name in ["g", "chev", "id", "n", "extent", "track", "span", "sum"] {
        assert!(
            names.iter().any(|found| found == name),
            "golden lacks {name}"
        );
    }
    for name in names.into_iter().filter(|name| name != "label") {
        assert!(
            body.contains(&format!("class: \"{name}\"")),
            "script lacks class: \"{name}\""
        );
    }
}

#[test]
fn the_script_draws_the_same_chevron_and_extent_bar_as_the_rust_tree() {
    let body = script();
    for shared in [
        "0 0 8 8",
        "M2.5 1l3.5 3-3.5 3",
        "0 0 100 6",
        "preserveAspectRatio",
        "none",
    ] {
        assert!(TREE_EXTENT.contains(shared), "golden lacks {shared}");
        assert!(body.contains(shared), "script lacks {shared}");
    }
    for fragment in [
        "function extentSpan(lo, hi, total) {",
        "if (![lo, hi, total].every(Number.isFinite) || total <= 0 || lo >= hi || lo >= total) return null;",
        "Math.max(((Math.min(hi, total) - lo) * 100) / total, EXTENT_MIN)",
        "Math.min((lo * 100) / total, 100 - width)",
    ] {
        assert!(body.contains(fragment), "script lacks {fragment}");
    }
    assert!(body.contains("const EXTENT_MIN = 1;"));
    assert!(body.contains("function oneLine(text)"));
    assert!(body.contains("replace(/\\s+/g, \" \")"));
}

#[test]
fn a_pending_node_gets_the_neutral_pill_not_a_sentence_in_its_row() {
    let body = script();
    assert!(TREE_PENDING
        .contains("<r-pill tone=\"neutral\" shape=\"square\"><span>pending</span></r-pill>"));
    assert!(body.contains("\"r-pill\""));
    assert!(body.contains("tone: \"neutral\""));
    assert!(body.contains("shape: \"square\""));
    assert!(!body.contains("node.summary === null ? \"summary pending\""));
}

#[test]
fn the_tree_keeps_the_selectors_the_keyboard_navigation_uses() {
    let body = script();
    assert!(body.contains("li[role=\"treeitem\"] > button"));
    assert!(body.contains(":scope > ul[role=\"group\"]"));
    assert!(body.contains(".firstElementChild.focus()"));
    let build = &body[body.find("function treeItem(").unwrap()..];
    let button = build.find("append(item, button").unwrap();
    let group = build.find("el(\"ul\", null, { role: \"group\" })").unwrap();
    assert!(button < group);
}

#[test]
fn the_script_makes_svg_with_the_svg_namespace_and_no_inline_style() {
    let body = script();
    assert!(body.contains("createElementNS"));
    assert!(!body.contains("data:"));
    assert!(!body.contains("setAttribute(\"style\""));
}

#[test]
fn the_memory_header_kv_always_uses_the_two_variant_without_a_source() {
    let body = script();
    assert!(body.contains("function kv(pairs)"));
    assert!(body.contains("el(\"r-kv\", null, { two: \"\" })"));
    assert!(body.contains("kv(detailPairs("));
    assert!(!body.contains("source"));
}

#[test]
fn the_entry_text_is_sent_whole_with_no_character_cut() {
    let body = script();
    assert!(!body.contains("PREVIEW"));
    assert!(!body.contains("function preview("));
    assert!(!body.contains("slice(0, max"));
    let start = body.find("function firstLine(text) {").unwrap();
    let end = start + body[start..].find("\n}\n").unwrap();
    let first_line = &body[start..end];
    assert!(first_line.contains(r".split(/\r?\n/)"), "{first_line}");
    assert!(
        first_line.contains(r#".find((part) => part.trim() !== "")"#),
        "{first_line}"
    );
    assert!(first_line.contains("line.trim()"), "{first_line}");
}

#[test]
fn the_entries_table_text_cell_is_a_clip_cell_holding_the_first_line() {
    let body = script();
    assert!(body.contains(r#"el("td", firstLine(entry.text), { class: "clip" })"#));
}

#[test]
fn an_empty_page_is_a_state_row_inside_the_table() {
    let body = script();
    assert!(body.contains("function stateRow(columns, child)"));
    assert!(body.contains(r#"el("tr", null, { class: "state" })"#));
    assert!(body.contains(r#"colspan: String(columns)"#));
    assert!(!body.contains(r#"return el("r-empty", "no entries in this page")"#));
}

#[test]
fn only_the_entries_table_is_stackable() {
    let body = script();
    assert!(body.contains(r#"el("r-table", null, { class: "stackable" })"#));
    assert_eq!(body.matches("stackable").count(), 1);
}

#[test]
fn the_entry_number_column_is_a_num_column_in_head_and_body() {
    let body = script();
    assert!(body.contains(r##"el("th", name, name === "#" ? { class: "num" } : {})"##));
    assert!(body.contains(r#"el("td", String(entry.i), { class: "num" })"#));
}

#[test]
fn first_line_reads_null_and_undefined_as_empty_text() {
    let body = script();
    assert!(body.contains(r#"String(text ?? "")"#));
}

fn function_body<'a>(body: &'a str, head: &str) -> &'a str {
    let start = body.find(head).unwrap_or_else(|| panic!("no {head}"));
    let end = start + body[start..].find("\n}\n").unwrap();
    &body[start..end]
}

#[test]
fn the_thread_select_is_replaced_by_tabs() {
    let body = script();
    assert!(!body.contains("threadSelect"));
    assert!(!body.contains("\"select\""));
    assert!(!body.contains("\"option\""));
    let tabs = function_body(&body, "function threadTabs(");
    assert!(tabs.contains(r#"el("nav", null, { class: "tabs", "aria-label": "threads" })"#));
    assert!(tabs.contains(r#""aria-current": "page""#));
    assert!(tabs.contains("pill(note)"));
    assert!(tabs.contains("meta.parked"));
    assert!(tabs.contains("meta.merged"));
}

#[test]
fn the_stat_line_replaces_the_kv_box() {
    let body = script();
    assert!(body.contains(r#"el("p", statLine(data.roots.stats), { class: "statline" })"#));
    let stat = function_body(&body, "function statLine(");
    for field in ["entries", "blocks", "summarised", "pending", "last_at"] {
        assert!(stat.contains(field), "{field}");
    }
    assert!(!body.contains(r#"kv([["thread""#));
}

#[test]
fn the_screen_is_a_tree_beside_a_detail_pane() {
    let body = script();
    assert!(body.contains(r#"el("div", null, { class: "split" })"#));
    assert!(body.contains(r#"el("div", null, { class: "detail" })"#));
    assert!(body.contains(r#""aria-selected", "true""#));
    assert!(body.contains("history.replaceState(null, \"\", buildHash("));
    assert_eq!(body.matches("history.replaceState").count(), 1);
}

#[test]
fn the_zoom_adds_no_summary_row_to_the_tree_group() {
    let body = script();
    let zoom = function_body(&body, "function zoomRows(zoom)");
    assert!(!zoom.contains("summary"));
    assert!(zoom.contains("zoom.children.map("));
    let open = function_body(&body, "async function openNode(");
    assert!(!open.contains("summary"));
    assert!(!open.contains("fillRendered"));
    assert!(!body.contains("row: \"summary\""));
}

#[test]
fn the_script_never_writes_summary_pending_into_a_row() {
    let body = script();
    assert!(!body.contains("summary pending"));
    assert!(!body.contains("r-empty\", \"summary"));
}

#[test]
fn the_breadcrumb_is_built_from_the_api_path() {
    let body = script();
    let parts = function_body(&body, "function crumbParts(");
    assert!(parts.contains("path.map("));
    assert!(parts.contains("rangeLabel("));
    assert!(parts.contains("buildHash("));
    let crumbs = function_body(&body, "function crumbs(");
    assert!(crumbs.contains(r#"el("nav", null, { class: "crumbs", "aria-label": "breadcrumb" })"#));
    assert!(crumbs.contains(r#""aria-current": "page""#));
    assert!(body.contains("crumbParts(state.thread, zoom ? zoom.path : [])"));
    assert!(body.contains("function rangeLabel(lo, hi)"));
    assert!(body.contains(r#""[" + lo + "-" + hi + "]""#));
}

#[test]
fn a_leaf_root_node_is_drawn_as_the_entry() {
    let body = script();
    let rows = function_body(&body, "function rootRows(");
    assert!(rows.contains("node.leaf && node.entry"));
    assert!(rows.contains(r#"row: "entry", child: node.entry"#));
    assert!(body.contains("rootRows(roots).forEach((row) => append(list, drawRow(1, row, total)))"));
}

#[test]
fn a_folded_block_has_a_pending_count_pill_and_no_expansion() {
    let body = script();
    let label = function_body(&body, "function pendingLabel(");
    assert!(label.contains("node.folded"));
    assert!(label.contains("\"pending ×\" + node.stats.pending"));
    let item = function_body(&body, "function nodeItem(");
    assert!(item.contains("!node.folded"));
    let tree = function_body(&body, "function treeItem(");
    assert!(tree.contains(r#"if (expandable) item.setAttribute("aria-expanded", "false");"#));
    assert!(tree.contains(r#"if (expandable) append(item, el("ul", null, { role: "group" }));"#));
    assert!(tree.contains("if (expandable) append(button, chevron());"));
}
