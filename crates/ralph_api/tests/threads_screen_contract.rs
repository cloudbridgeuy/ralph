#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]
#[allow(dead_code, reason = "each test crate uses a subset of the helpers")]
mod common;

use common::{Host, Reply, TestServer};

const APP_JS: &str = include_str!("../assets/app.js");
const THREADS_JS: &str = include_str!("../assets/threads.js");
const THREADS_CSS: &str = include_str!("../assets/css/threads.css");

fn get(server: &TestServer, target: &str) -> Reply {
    let cookie = format!("{}={}", server.cookie_name(), server.token());
    server.exchange("GET", target, Host::Own, &[("Cookie", cookie.as_str())])
}

fn served(target: &str) -> String {
    let server = TestServer::start();
    let reply = get(&server, target);
    assert_eq!(reply.status, 200, "{target}");
    reply.body
}

fn script() -> String {
    served("/app.js")
}

fn function_body<'a>(body: &'a str, head: &str) -> &'a str {
    let start = body.find(head).unwrap_or_else(|| panic!("no {head}"));
    let end = start + body[start..].find("\n}\n").unwrap();
    &body[start..end]
}

#[test]
fn the_threads_script_is_joined_into_the_single_app_js() {
    let body = script();
    assert_eq!(body, format!("{THREADS_JS}{APP_JS}"));
    assert_eq!(body.matches("async function showThreads(").count(), 1);
    assert_eq!(body.matches("\nasync function show(").count(), 1);
    let server = TestServer::start();
    assert_eq!(get(&server, "/threads.js").status, 404);
    assert_eq!(get(&server, "/assets/threads.js").status, 404);
}

#[test]
fn the_threads_script_opens_in_strict_mode_and_declares_functions_before_app_js_runs_show() {
    assert!(THREADS_JS.starts_with("\"use strict\";\n"));
    assert!(THREADS_JS.ends_with("\n"));
    assert!(APP_JS.trim_end().ends_with("show();"));
    assert!(!THREADS_JS.contains("show();"));
}

#[test]
fn the_nav_lists_threads_between_memory_and_config() {
    let html = served("/");
    let memory = html
        .find("<a href=\"#/memory\"><span class=\"n\" aria-hidden=\"true\">1</span>memory</a>")
        .unwrap();
    let threads = html
        .find("<a href=\"#/threads\"><span class=\"n\" aria-hidden=\"true\">2</span>threads</a>")
        .unwrap();
    let config = html
        .find("<a href=\"#/config\"><span class=\"n\" aria-hidden=\"true\">3</span>config</a>")
        .unwrap();
    assert!(memory < threads && threads < config);
    assert!(APP_JS.contains("const SCREENS = [\"/memory\", \"/threads\", \"/config\"];"));
    assert!(APP_JS.contains("const ROUTE = SCREENS[0];"));
}

#[test]
fn the_show_function_hands_the_threads_route_to_the_threads_screen() {
    let show = function_body(APP_JS, "async function show(refocus)");
    assert!(show
        .contains("if (state.route === THREADS_ROUTE) return showThreads(main, mine, refocus);"));
    let hand_off = show.find("showThreads(").unwrap();
    assert!(hand_off > show.find("markCurrentScreen(state.route);").unwrap());
    assert!(hand_off < show.find("const config = ").unwrap());
}

#[test]
fn the_threads_screen_reuses_focus_retry_and_generation_guard() {
    let show = function_body(THREADS_JS, "async function showThreads(");
    assert!(show.contains("mine !== generation"));
    assert_eq!(show.matches("mine !== generation").count(), 2);
    assert!(show.contains("delayedSpinner(main, \"Loading threads\\u2026\")"));
    assert!(show.contains("await loadThreads()"));
    assert!(show.contains("threadsFailure(error, () => show(true))"));
    assert!(show.contains("if (refocus) focusRetry(box);"));
    assert!(show.contains("if (refocus) focusTarget(main);"));
    assert!(THREADS_JS.contains("getJson(\"/api/threads\", {})"));
    assert!(THREADS_JS.contains("document.getElementById(\"store\").textContent = data.store;"));
}

#[test]
fn the_threads_script_adds_no_inner_html_and_takes_the_graph_from_the_server() {
    assert!(!THREADS_JS.contains("innerHTML"));
    assert!(THREADS_JS.contains("setRendered(host, row.svg)"));
    for geometry in ["r-graph", "createElementNS", "3.25", "viewBox=\"0 0 24"] {
        assert!(!THREADS_JS.contains(geometry), "{geometry}");
    }
    for banned in ["eval(", "style.", "unsafe", "token", "new Function"] {
        assert!(!THREADS_JS.contains(banned), "{banned}");
    }
}

#[test]
fn the_threads_rows_keep_head_and_dropped_out_of_the_keyboard() {
    assert!(
        THREADS_JS.contains("function threadPickable(row) {\n  return row.kind === \"thread\";\n}")
    );
    let row = function_body(THREADS_JS, "function threadRow(");
    assert!(row.contains("threadPickable(row)"));
    assert!(row.contains("{ class: \"quiet\" }"));
    assert!(row.contains("tabindex: \"0\""));
    assert_eq!(THREADS_JS.matches("tabindex").count(), 1);
    let rows = function_body(THREADS_JS, "function threadPick(");
    assert!(rows.contains("rows.filter(threadPickable)"));
    assert!(!THREADS_JS.contains("const THREAD_ORDER_KINDS"));
}

#[test]
fn the_default_selection_is_the_first_selectable_row_and_the_hash_stays() {
    let selection = function_body(THREADS_JS, "function threadPick(");
    assert!(selection
        .contains("if (name === null) return { row: choices[0] || null, missing: false };"));
    assert!(selection.contains("const row = choices.find((choice) => choice.name === name);"));
    assert!(selection.contains("return { row: row || null, missing: !row };"));
    let draw = function_body(THREADS_JS, "function drawThreads(");
    assert!(!draw.contains("replaceState"));
    let select = function_body(THREADS_JS, "function pickThread(");
    assert_eq!(
        select
            .matches("history.replaceState(null, \"\", threadsHash(row.name));")
            .count(),
        1
    );
}

#[test]
fn the_thread_param_is_null_when_absent_so_the_default_never_hides_a_stale_link() {
    assert!(THREADS_JS.contains("function threadParam(hash) {"));
    assert!(THREADS_JS.contains(
        "return cut < 0 ? null : new URLSearchParams(hash.slice(cut + 1)).get(\"thread\") || null;"
    ));
    assert!(THREADS_JS.contains("function threadsHash(name) {"));
    assert!(THREADS_JS.contains("threadParam(location.hash)"));
}

#[test]
fn the_table_has_eight_columns_with_the_designed_drop_classes() {
    for fragment in [
        "const THREAD_COLUMNS = 8;",
        "{ label: \"name\", class: \"gname\" }",
        "{ label: \"state\" }",
        "{ label: \"fork\", class: \"num\" }",
        "{ label: \"created\" }",
        "{ label: \"active\", class: \"drop-1\" }",
        "{ label: \"msgs\", class: \"num\" }",
        "{ label: \"workers\", class: \"num drop-3\" }",
        "{ label: \"unread\", class: \"num drop-2\" }",
    ] {
        assert!(THREADS_JS.contains(fragment), "{fragment}");
    }
    assert!(THREADS_JS.contains("class: \"threads\""));
    assert!(THREADS_JS.contains("\"aria-label\": \"threads\""));
}

#[test]
fn zero_counts_show_a_muted_dash_and_unread_above_zero_is_hot() {
    assert!(THREADS_JS.contains("function threadCount(value) {"));
    assert!(THREADS_JS.contains("el(\"span\", \"\\u2013\", { class: \"muted\" })"));
    assert!(!THREADS_CSS.contains("--r-faint"));
    assert!(THREADS_JS.contains("el(\"span\", String(row.unread), { class: \"hot\" })"));
    assert!(THREADS_JS.contains("row.unread > 0"));
}

#[test]
fn the_state_pills_follow_the_design() {
    for fragment in [
        "open: { tone: \"neutral\", icon: \"M8 5.5A2.5 2.5 0 1 1 8 10.5A2.5 2.5 0 1 1 8 5.5Z\" }",
        "parked: { tone: \"warn\", icon: \"M6 4V12M10 4V12\" }",
        "merged: { tone: \"ok\", icon: \"M3 8L6.5 11.5L13 4.5\" }",
        "dropped: { tone: \"bad\", icon: \"M4 4L12 12M12 4L4 12\", variant: \"outline\" }",
    ] {
        assert!(THREADS_JS.contains(fragment), "{fragment}");
    }
    let state = function_body(THREADS_JS, "function stateCell(");
    assert!(state.contains("class: \"age\""));
    assert!(state.contains("row.age"));
    assert!(state.contains("\"head\""));
}

#[test]
fn the_head_row_and_the_dropped_rows_have_no_fork_created_or_counts() {
    let row = function_body(THREADS_JS, "function threadRow(");
    assert!(row.contains("row.kind === \"head\""));
    assert!(row.contains("row.kind === \"thread\""));
    assert!(row.contains("class: \"nm strong\""));
    assert!(row.contains("class: \"nm\""));
}

#[test]
fn the_fork_cell_marks_the_hash_in_muted_and_the_created_cell_keeps_the_date() {
    assert!(THREADS_JS.contains("el(\"span\", \"#\", { class: \"muted\" })"));
    assert!(THREADS_JS.contains("function createdParts(iso) {"));
    assert!(THREADS_JS.contains("const match = AT_SHAPE.exec(String(iso));"));
    assert!(THREADS_JS.contains(
        "return match ? { date: match[1], time: match[2] } : { date: String(iso), time: \"\" };"
    ));
}

#[test]
fn the_detail_pane_lists_the_designed_pairs_and_two_rules() {
    let detail = function_body(THREADS_JS, "function threadDetail(");
    for fragment in [
        "[\"state\", ",
        "[\"fork\", ",
        "[\"created\", ",
        "[\"active\", ",
        "[\"messages\", ",
        "[\"workers\", ",
        "[\"unread\", ",
        "class: \"aside\"",
        "\" messages\"",
        "\"memory\"",
        "\"Open the memory of \" + row.name + \" \\u2192\"",
        "buildHash({ thread: row.name })",
        "el(\"kbd\", \"enter\")",
        "\"in the terminal\"",
        "class: \"cmds\"",
        "class: \"pane-link\"",
    ] {
        assert!(detail.contains(fragment), "{fragment}");
    }
    assert!(THREADS_JS.contains("el(\"span\", \"thread\", { class: \"sub\" })"));
    assert!(THREADS_JS.contains("\"of \" + head.messages + \" on main\""));
    assert!(THREADS_JS
        .contains("extentCell({ lo: row.base, hi: head.messages, total: head.messages })"));
}

#[test]
fn each_state_offers_its_terminal_commands() {
    let commands = function_body(THREADS_JS, "function threadCommands(");
    for fragment in [
        "open: [\"ralph chat --thread \" + name, \"ralph thread pull \" + name, \"ralph thread merge \" + name]",
        "parked: [\"ralph thread revive \" + name, \"ralph thread drop \" + name]",
        "merged: []",
    ] {
        assert!(commands.contains(fragment), "{fragment}");
    }
    assert!(THREADS_JS.contains("el(\"span\", text, { class: \"cmd\" })"));
}

#[test]
fn the_whole_screen_error_names_the_status_what_and_why_and_offers_retry() {
    let failure = function_body(THREADS_JS, "function threadsFailure(");
    for fragment in [
        "r-error",
        "role: \"alert\"",
        "\"The threads screen did not load.\"",
        "String(error.status)",
        "threadWhy(error)",
        "el(\"kbd\", \"r\")",
        "\"Retry\"",
        "button.addEventListener(\"click\", retry);",
    ] {
        assert!(failure.contains(fragment), "{fragment}");
    }
    assert!(THREADS_JS.contains(
        "\"The server answered busy: \" + SERVER_HANDLERS + \" requests are in progress.\""
    ));
    assert!(THREADS_JS.contains("const SERVER_HANDLERS = 8;"));
    assert!(THREADS_JS.contains("const BUSY_STATUS = 503;"));
    let why = function_body(THREADS_JS, "function threadWhy(");
    assert!(why.contains("error.status === BUSY_STATUS"));
    assert!(why.contains("(error && error.message) || String(error)"));
}

#[test]
fn an_unknown_thread_keeps_the_table_and_explains_in_the_detail_pane() {
    let missing = function_body(THREADS_JS, "function threadsMissing(");
    for fragment in [
        "\"404\"",
        "\"Thread \"",
        "el(\"strong\", name)",
        "\" was not found.\"",
        "\"The link is old, or the thread was dropped. The table stays in view and no row is selected.\"",
        "\"Open a thread that exists:\"",
        "MISSING_CHOICES",
        "threadsHash(choice.name)",
    ] {
        assert!(missing.contains(fragment), "{fragment}");
    }
    assert!(THREADS_JS.contains("const MISSING_CHOICES = 3;"));
    let draw = function_body(THREADS_JS, "function drawThreads(");
    assert!(draw.contains("selection.missing"));
}

#[test]
fn the_empty_state_names_the_store_and_the_chat_command() {
    let empty = function_body(THREADS_JS, "function threadsEmpty(");
    for fragment in [
        "emptyState(\"no threads in \")",
        "el(\"code\", store, { class: \"cmd-inline\" })",
        "\"Open a thread from main:\"",
        "\"ralph chat --thread <name>\"",
        "class: \"cmd\"",
    ] {
        assert!(empty.contains(fragment), "{fragment}");
    }
    let draw = function_body(THREADS_JS, "function drawThreads(");
    assert!(draw.contains("stateRow(THREAD_COLUMNS, threadsEmpty(data.store))"));
    assert!(draw.contains("choices.length === 0"));
}

#[test]
fn the_box_title_carries_the_store_and_the_aside_carries_the_key_hints() {
    let draw = function_body(THREADS_JS, "function drawThreads(");
    assert!(draw.contains("el(\"r-box\", null, { flush: \"\" })"));
    assert!(draw.contains("el(\"span\", data.store, { class: \"sub\" })"));
    let keys = function_body(THREADS_JS, "function keyHints(");
    for fragment in [
        "\"\\u2191\\u2193\"",
        "\"move\"",
        "\"home\"",
        "\"end\"",
        "\"enter\"",
        "\"memory\"",
        "class: \"keys\"",
        "class: \"aside hide-narrow\"",
    ] {
        assert!(keys.contains(fragment), "{fragment}");
    }
}

#[test]
fn the_detail_pane_and_table_sit_in_the_master_wide_split() {
    let draw = function_body(THREADS_JS, "function drawThreads(");
    assert!(draw.contains("class: \"split master-wide\""));
    assert!(draw.contains("crumbs(threadsCrumbParts("));
    let parts = function_body(THREADS_JS, "function threadsCrumbParts(");
    assert!(parts.contains("label: \"threads\""));
    assert!(parts.contains("hash: threadsHash(null)"));
}

#[test]
fn the_enter_key_opens_the_memory_of_the_selected_thread() {
    let keys = function_body(THREADS_JS, "function onThreadsKey(");
    assert!(keys.contains("event.key === \"Enter\""));
    assert!(keys.contains("location.hash = buildHash({ thread: "));
    assert!(THREADS_JS.contains("document.addEventListener(\"keydown\", onThreadsKey);"));
}

#[test]
fn enter_and_space_act_on_the_focused_row_and_enter_falls_back_to_the_selected_row() {
    let keys = function_body(THREADS_JS, "function onThreadsKey(");
    assert!(keys.contains("event.target.closest(\"tr[data-name]\")"));
    assert!(keys.contains("event.key === \" \""));
    assert!(keys.contains("event.key === \"Enter\""));
    assert!(keys.contains("row.click();"));
    assert!(keys.contains("location.hash = buildHash({ thread: row.dataset.name });"));
    assert!(!keys.contains("rows[at].dataset.name"));
}

#[test]
fn r_retries_even_while_focus_is_on_the_retry_button_and_other_keys_still_skip_buttons() {
    let keys = function_body(THREADS_JS, "function onThreadsKey(");
    let retry = keys.find("event.key === \"r\"").unwrap();
    let typing = keys.find("isTyping(event.target)").unwrap();
    assert!(retry < typing);
    assert!(keys.contains(
        "event.key === \"r\" && (!isTyping(event.target) || event.target.closest(\"r-error\"))"
    ));
    assert!(keys.contains("if (isTyping(event.target) || event.target.closest(\"a\")) return;"));
    assert_eq!(keys.matches("isTyping(event.target)").count(), 2);
}

#[test]
fn the_keyboard_moves_only_over_selectable_rows_and_ignores_modifiers_and_other_keys() {
    let keys = function_body(THREADS_JS, "function onThreadsKey(");
    assert!(
        keys.contains("event.ctrlKey || event.metaKey || event.altKey || event.defaultPrevented")
    );
    assert!(keys.contains("isTyping(event.target)"));
    assert!(keys.contains("event.target.closest(\"a\")"));
    assert!(keys.contains("threadMove(event.key, "));
    let step = function_body(THREADS_JS, "function threadMove(");
    for fragment in ["\"ArrowDown\"", "\"ArrowUp\"", "\"Home\"", "\"End\""] {
        assert!(step.contains(fragment), "{fragment}");
    }
    assert!(!THREADS_JS.contains("Escape"));
    assert!(!THREADS_JS.contains("statusbar"));
}

#[test]
fn selecting_a_row_updates_the_hash_the_crumb_and_the_detail_pane() {
    let select = function_body(THREADS_JS, "function pickThread(");
    assert!(select.contains("\"aria-selected\", \"true\""));
    assert!(select.contains("removeAttribute(\"aria-selected\")"));
    assert!(
        select.contains("screen.pane.replaceWith(pane)")
            || select.contains("screen.pane.replaceChildren(")
    );
    assert!(select.contains("threadsCrumbParts(row.name)"));
}

#[test]
fn the_stat_line_counts_threads_by_state_and_the_messages_on_main() {
    let stats = function_body(THREADS_JS, "function threadStats(");
    for fragment in [
        "\" threads, \"",
        "\" open, \"",
        "\" parked, \"",
        "\" merged, \"",
        "\" dropped, \"",
        "\" messages on main\"",
    ] {
        assert!(stats.contains(fragment), "{fragment}");
    }
}

#[test]
fn the_loading_text_and_the_error_strings_use_the_ellipsis_escape() {
    assert!(THREADS_JS.contains("Loading threads\\u2026"));
    assert!(!THREADS_JS.contains('\u{2026}'));
}

#[test]
fn the_threads_screen_script_stays_under_1000_lines() {
    assert!(THREADS_JS.lines().count() < 1000);
    assert!(APP_JS.lines().count() < 1000);
    assert!(THREADS_CSS.lines().count() < 1000);
}

fn css_text() -> String {
    served("/assets/app.css")
}

#[test]
fn threads_css_uses_role_tokens_only() {
    for banned in ["#", "rgb", "hsl", "@font-face", "url(", "font-family"] {
        assert!(!THREADS_CSS.contains(banned), "{banned}");
    }
    assert!(THREADS_CSS.contains("var(--r-"));
    for line in THREADS_CSS.lines().map(str::trim) {
        assert!(!line.starts_with("--"), "{line}");
        assert!(!line.contains("var(--t)"), "{line}");
    }
    for token in THREADS_CSS.split("var(--").skip(1) {
        assert!(token.starts_with("r-"), "{token}");
    }
}

#[test]
fn the_served_sheet_ends_with_threads_css_after_responsive_and_memory() {
    let css = css_text();
    assert!(css.ends_with(THREADS_CSS));
    let threads = css.rfind(THREADS_CSS).unwrap();
    assert!(css.find("r-table.stackable thead").unwrap() < threads);
    assert!(css.find("r-tree li > button:focus-visible").unwrap() < threads);
}

#[test]
fn the_threads_split_is_scoped_and_stacks_at_760() {
    assert!(THREADS_CSS.contains(".split.master-wide {"));
    assert!(THREADS_CSS.contains("grid-template-columns: minmax(0, 3fr) minmax(40ch, 2fr);"));
    assert!(!THREADS_CSS.contains("\n.split {"));
    let narrow = &THREADS_CSS[THREADS_CSS.find("@media (max-width: 760px)").unwrap()..];
    assert!(narrow.contains(".split.master-wide {"));
    assert!(narrow.contains("grid-template-columns: minmax(0, 1fr);"));
    assert!(narrow.contains("r-table.threads .age {"));
}

#[test]
fn the_columns_drop_at_the_designed_widths() {
    for (width, class) in [(1180, ".drop-1"), (1000, ".drop-2"), (760, ".drop-3")] {
        let query = format!("@media (max-width: {width}px)");
        let at = THREADS_CSS
            .find(&query)
            .unwrap_or_else(|| panic!("{query}"));
        let rule = &THREADS_CSS[at..];
        let rule = &rule[..rule.find("}\n").unwrap()];
        assert!(rule.contains(class), "{class}");
        assert!(rule.contains("display: none;"), "{class}");
    }
}

#[test]
fn quiet_rows_have_no_hover_and_selected_rows_use_the_selected_fill() {
    assert!(THREADS_CSS.contains("r-table.threads tbody tr.quiet:hover > td {"));
    assert!(THREADS_CSS.contains("background: none;"));
    assert!(THREADS_CSS.contains("r-table.threads tbody tr:focus-visible {"));
    assert!(THREADS_CSS.contains("outline: 2px solid var(--r-accent);"));
    assert!(THREADS_CSS.contains("outline-offset: -2px;"));
}

#[test]
fn the_chips_reuse_the_command_chip_look() {
    assert!(THREADS_CSS.contains("\n.cmd {"));
    assert!(THREADS_CSS.contains("\n.cmd-inline {"));
    assert!(THREADS_CSS.contains(".cmd::before {"));
    assert!(THREADS_CSS.contains("content: \"$ \";"));
    assert!(THREADS_CSS.contains("\n.muted {"));
}
