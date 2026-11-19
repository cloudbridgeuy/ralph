use super::*;
use ralph_core::chat_memory::{parse_tool_call, CallMetrics, Tool};
use serde_json::{json, Value};

const STAMP: &str = "2026-01-01T00:00";

fn seeded(texts: &[String]) -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("s"));
    fs::create_dir_all(store.dir()).unwrap();
    let mut log = String::new();
    for (i, text) in texts.iter().enumerate() {
        let entry = NewEntry {
            role: if i % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            },
            at: format!("{STAMP}:{i:02}Z"),
            text: text.clone(),
        };
        log.push_str(&encode_entry(&entry).unwrap());
    }
    fs::write(store.dir().join("log.jsonl"), log).unwrap();
    (dir, store)
}

fn tool(name: &str, args: Value) -> Tool {
    parse_tool_call(name, &args).unwrap().tool
}

fn run(store: &Store, name: &str, args: Value) -> Result<String, String> {
    store
        .call(&tool(name, args), Reach::default())
        .map(|answer| answer.text)
}

fn long_text() -> String {
    "long ".repeat(200)
}

fn small_store() -> (tempfile::TempDir, Store) {
    let texts = [
        "alpha one".to_string(),
        "beta needle two".to_string(),
        long_text(),
        "gamma needle".to_string(),
        "delta".to_string(),
        "epsilon needle".to_string(),
        "zeta".to_string(),
        "eta".to_string(),
    ];
    let (dir, store) = seeded(&texts);
    store.put(Node::new(0, 2).unwrap(), "pair summary").unwrap();
    store.put(Node::new(2, 3).unwrap(), "leaf summary").unwrap();
    (dir, store)
}

#[test]
fn zoom_of_a_block_lists_its_halves() {
    let (_dir, store) = small_store();
    assert_eq!(
        run(&store, "zoom", json!({"id": "0-3"})).unwrap(),
        "#0-1 pair summary\n#2-3 not compressed yet"
    );
    assert_eq!(
        run(&store, "zoom", json!({"id": "0-7"})).unwrap(),
        "#0-3 not compressed yet\n#4-7 not compressed yet"
    );
}

#[test]
fn zoom_of_a_message_prints_it_whole() {
    let (_dir, store) = small_store();
    assert_eq!(
        run(&store, "zoom", json!({"id": "1"})).unwrap(),
        "#1 assistant 2026-01-01T00:00:01Z beta needle two"
    );
    assert_eq!(
        run(&store, "zoom", json!({"id": "0-1"})).unwrap(),
        "#0 user 2026-01-01T00:00:00Z alpha one\n#1 assistant 2026-01-01T00:00:01Z beta needle two"
    );
}

#[test]
fn zoom_and_read_beyond_the_chat_fail_with_the_size() {
    let (_dir, store) = small_store();
    assert_eq!(
        run(&store, "zoom", json!({"id": "20"})).unwrap_err(),
        "#20 is beyond the chat: it holds 8 entries."
    );
    assert_eq!(
        run(&store, "read", json!({"id": "20-30"})).unwrap_err(),
        "#20 is beyond the chat: it holds 8 entries."
    );
}

#[test]
fn read_raw_prints_entries_in_full() {
    let (_dir, store) = small_store();
    let expected = format!(
        "#0 · user · {STAMP}:00Z · 9 chars\nalpha one\n\n\
         #1 · assistant · {STAMP}:01Z · 15 chars\nbeta needle two\n\n\
         #2 · user · {STAMP}:02Z · 1000 chars\n{}\n\n\
         #3 · assistant · {STAMP}:03Z · 12 chars\ngamma needle\n\n\
         (end of range)",
        long_text()
    );
    assert_eq!(
        run(&store, "read", json!({"id": "0-3", "mode": "raw"})).unwrap(),
        expected
    );
}

#[test]
fn read_summary_swaps_long_entries_for_their_summary() {
    let (_dir, store) = small_store();
    let expected = format!(
        "#0 · user · {STAMP}:00Z · 9 chars\nalpha one\n\n\
         #1 · assistant · {STAMP}:01Z · 15 chars\nbeta needle two\n\n\
         #2 · user · {STAMP}:02Z · 12 chars\nleaf summary\n\n\
         #3 · assistant · {STAMP}:03Z · 12 chars\ngamma needle\n\n\
         (end of range)"
    );
    assert_eq!(
        run(&store, "read", json!({"id": "0-3", "mode": "summary"})).unwrap(),
        expected
    );
}

#[test]
fn read_pages_with_a_cursor() {
    let (_dir, store) = small_store();
    let first = run(
        &store,
        "read",
        json!({"id": "0-7", "mode": "raw", "budget": 400}),
    )
    .unwrap();
    let expected_first = format!(
        "#0 · user · {STAMP}:00Z · 9 chars\nalpha one\n\n\
         #1 · assistant · {STAMP}:01Z · 15 chars\nbeta needle two\n\n\
         #2 · user · {STAMP}:02Z · 1000 chars\n{}\n\n\
         next cursor: 3.0@8r",
        long_text()
    );
    assert_eq!(first, expected_first);
    let second = run(
        &store,
        "read",
        json!({"id": "0-7", "mode": "raw", "budget": 400, "cursor": "3.0@8r"}),
    )
    .unwrap();
    let expected_second = format!(
        "#3 · assistant · {STAMP}:03Z · 12 chars\ngamma needle\n\n\
         #4 · user · {STAMP}:04Z · 5 chars\ndelta\n\n\
         #5 · assistant · {STAMP}:05Z · 14 chars\nepsilon needle\n\n\
         #6 · user · {STAMP}:06Z · 4 chars\nzeta\n\n\
         #7 · assistant · {STAMP}:07Z · 3 chars\neta\n\n\
         (end of range)"
    );
    assert_eq!(second, expected_second);
}

#[test]
fn recall_lists_hits_newest_first() {
    let (_dir, store) = small_store();
    assert_eq!(
        run(&store, "recall", json!({"pattern": "needle"})).unwrap(),
        "#5 · assistant · 2026-01-01 · epsilon needle\n\
         #3 · assistant · 2026-01-01 · gamma needle\n\
         #1 · assistant · 2026-01-01 · beta needle two\n\
         (zoom <id> for the full text)"
    );
    assert_eq!(
        run(&store, "recall", json!({"pattern": "zzz"})).unwrap(),
        "no hits for zzz"
    );
    assert_eq!(
        run(&store, "recall", json!({"pattern": "("})).unwrap_err(),
        "bad regex: regex parse error:\n    (\n    ^\nerror: unclosed group"
    );
}

#[test]
fn recall_caps_the_listing_at_fifty_lines() {
    let texts: Vec<String> = (0..55).map(|i| format!("hit {i}")).collect();
    let (_dir, store) = seeded(&texts);
    let out = run(&store, "recall", json!({"pattern": "hit"})).unwrap();
    let mut expected: Vec<String> = (5..55)
        .rev()
        .map(|i| {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            format!("#{i} · {role} · 2026-01-01 · hit {i}")
        })
        .collect();
    expected.push("5 more, narrow the pattern".to_string());
    expected.push("(zoom <id> for the full text)".to_string());
    assert_eq!(out, expected.join("\n"));
}

#[test]
fn search_ranks_hits_and_reports_empty_results() {
    let (_dir, store) = small_store();
    assert_eq!(
        run(&store, "search", json!({"query": "needle"})).unwrap(),
        "#5 · assistant · 2026-01-01T00:00:05Z · epsilon needle\n\
         #3 · assistant · 2026-01-01T00:00:03Z · gamma needle\n\
         #1 · assistant · 2026-01-01T00:00:01Z · beta needle two\n\
         (end of results)\n\
         (read <id> for the full text)"
    );
    assert_eq!(
        run(&store, "search", json!({"query": "zzz"})).unwrap(),
        "no hits for zzz"
    );
}

#[test]
fn search_pages_with_a_cursor() {
    let (_dir, store) = small_store();
    assert_eq!(
        run(&store, "search", json!({"query": "needle", "limit": 1})).unwrap(),
        "#5 · assistant · 2026-01-01T00:00:05Z · epsilon needle\n\
         next cursor: 1.8.5f2190a9be0355a6\n\
         (read <id> for the full text)"
    );
    assert_eq!(
        run(
            &store,
            "search",
            json!({"query": "needle", "limit": 1, "cursor": "1.8.5f2190a9be0355a6"})
        )
        .unwrap(),
        "#3 · assistant · 2026-01-01T00:00:03Z · gamma needle\n\
         next cursor: 2.8.5f2190a9be0355a6\n\
         (read <id> for the full text)"
    );
    assert_eq!(
        run(
            &store,
            "search",
            json!({"query": "needle", "cursor": "1.2.3"})
        )
        .unwrap_err(),
        "cursor does not match this query"
    );
}

#[test]
fn a_reach_limit_hides_later_entries() {
    let (_dir, store) = small_store();
    let reach = Reach {
        limit: Some(4),
        label: None,
    };
    assert_eq!(
        store
            .call(&tool("recall", json!({"pattern": "needle"})), reach)
            .unwrap()
            .text,
        "#3 · assistant · 2026-01-01 · gamma needle\n\
         #1 · assistant · 2026-01-01 · beta needle two\n\
         (zoom <id> for the full text)"
    );
}

#[test]
fn a_missing_store_reads_as_empty() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("none"));
    assert_eq!(
        run(&store, "recall", json!({"pattern": "x"})).unwrap(),
        "no hits for x"
    );
    assert!(!store.dir().exists());
}

fn facts(store: &Store, name: &str, args: Value) -> CallMetrics {
    store
        .call(&tool(name, args), Reach::default())
        .unwrap()
        .facts
}

#[test]
fn zoom_facts_carry_node_size_and_depth() {
    let (_dir, store) = small_store();
    let block = facts(&store, "zoom", json!({"id": "0-3"}));
    assert_eq!((block.node_size, block.depth), (Some(4), Some(2)));
    assert_eq!((block.hits, block.top_cover_size), (None, None));
    let message = facts(&store, "zoom", json!({"id": "1"}));
    assert_eq!((message.node_size, message.depth), (Some(1), Some(0)));
    assert_eq!(message.in_view, None);
}

#[test]
fn read_facts_carry_the_requested_span() {
    let (_dir, store) = small_store();
    let page = facts(&store, "read", json!({"id": "0-3"}));
    assert_eq!((page.node_size, page.depth), (Some(4), None));
    assert_eq!((page.hits, page.oldest_hit), (None, None));
}

#[test]
fn recall_facts_count_hits_and_find_the_oldest_and_top_cover() {
    let (_dir, store) = small_store();
    let wide = facts(&store, "recall", json!({"pattern": "needle"}));
    assert_eq!(wide.hits, Some(3));
    assert_eq!(wide.truncated, Some(false));
    assert_eq!(wide.oldest_hit, Some(1));
    assert_eq!(wide.top_cover_size, Some(1));
    assert_eq!((wide.node_size, wide.in_view), (None, None));
    let covered = facts(&store, "recall", json!({"pattern": "alpha"}));
    assert_eq!(covered.top_cover_size, Some(2));
    let none = facts(&store, "recall", json!({"pattern": "zzz"}));
    assert_eq!(none.hits, Some(0));
    assert_eq!(none.truncated, Some(false));
    assert_eq!((none.oldest_hit, none.top_cover_size), (None, None));
}

#[test]
fn recall_facts_flag_the_fifty_line_cap() {
    let texts: Vec<String> = (0..55).map(|i| format!("hit {i}")).collect();
    let (_dir, store) = seeded(&texts);
    let capped = facts(&store, "recall", json!({"pattern": "hit"}));
    assert_eq!(capped.hits, Some(55));
    assert_eq!(capped.truncated, Some(true));
    assert_eq!(capped.oldest_hit, Some(0));
    assert_eq!(capped.top_cover_size, Some(1));
}

#[test]
fn search_facts_count_hits_and_find_the_top_cover() {
    let (_dir, store) = small_store();
    let wide = facts(&store, "search", json!({"query": "needle"}));
    assert_eq!(wide.hits, Some(3));
    assert_eq!(wide.truncated, None);
    assert_eq!(wide.oldest_hit, Some(1));
    assert_eq!(wide.top_cover_size, Some(1));
    let covered = facts(&store, "search", json!({"query": "alpha"}));
    assert_eq!(covered.hits, Some(1));
    assert_eq!(covered.top_cover_size, Some(2));
    let paged = facts(&store, "search", json!({"query": "needle", "limit": 1}));
    assert_eq!(paged.hits, Some(3));
    let none = facts(&store, "search", json!({"query": "zzz"}));
    assert_eq!(none.hits, Some(0));
    assert_eq!((none.oldest_hit, none.top_cover_size), (None, None));
}
