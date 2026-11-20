use super::{at, cursor_of, ids, run_tagged, timed, user, Bytes};
use crate::chat_memory::fixtures::written;
use crate::chat_memory::search::{parse_search_args, search_found};
use crate::chat_memory::{Archive, Marks, Role};
use serde_json::json;

const NOTICE: &str =
    "(tags exist only for messages written after tagging began; older messages are not listed)";

fn five() -> Vec<crate::chat_memory::Entry> {
    (0..5)
        .map(|i| user(i, &format!("note number {i}")))
        .collect()
}

#[test]
fn lists_tagged_entries_newest_first() {
    let out = run_tagged(&five(), json!({"query": "", "tag": "rule"}), &[0, 2, 3]).unwrap();
    assert_eq!(ids(&out), [3, 2, 0]);
    assert!(out.contains("note number 3"), "{out}");
}

#[test]
fn the_notice_is_the_last_line_after_the_read_hint() {
    let out = run_tagged(&five(), json!({"query": "", "tag": "rule"}), &[1]).unwrap();
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[lines.len() - 1], NOTICE, "{out}");
    assert_eq!(lines[lines.len() - 2], "(read <id> for the full text)");
}

#[test]
fn an_empty_listing_names_the_tag_and_keeps_the_notice() {
    let out = run_tagged(&five(), json!({"query": "", "tag": "rule"}), &[]).unwrap();
    assert_eq!(out, format!("no entries tagged rule\n{NOTICE}"));
}

#[test]
fn paging_covers_every_entry_once() {
    let entries = five();
    let args = |cursor: Option<String>| json!({"tag": "rule", "limit": 2, "cursor": cursor});
    let mut seen = Vec::new();
    let mut cursor = None;
    loop {
        let out = run_tagged(&entries, args(cursor), &[0, 1, 2, 3, 4]).unwrap();
        assert!(out.ends_with(NOTICE), "{out}");
        seen.extend(ids(&out));
        cursor = cursor_of(&out);
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(seen, [4, 3, 2, 1, 0]);
}

#[test]
fn role_and_since_narrow_the_listing() {
    let entries = [
        timed(Role::User, "2026-10-19T10:00:00.000000000+00:00", "one"),
        timed(
            Role::Assistant,
            "2026-10-20T10:00:00.000000000+00:00",
            "two",
        ),
        timed(Role::User, "2026-10-21T10:00:00.000000000+00:00", "three"),
    ];
    let all = [0, 1, 2];
    let by_role = run_tagged(&entries, json!({"tag": "rule", "role": "user"}), &all).unwrap();
    assert_eq!(ids(&by_role), [2, 0]);
    let by_date = run_tagged(
        &entries,
        json!({"tag": "rule", "since": "2026-10-20"}),
        &all,
    );
    assert_eq!(ids(&by_date.unwrap()), [2, 1]);
}

#[test]
fn a_decision_listing_is_user_only() {
    let entries = [user(0, "a"), timed(Role::Assistant, &at(1), "b")];
    let out = run_tagged(&entries, json!({"tag": "decision"}), &[0, 1]).unwrap();
    assert_eq!(ids(&out), [0]);
}

#[test]
fn no_words_and_no_tag_is_an_error() {
    for args in [
        json!({"query": ""}),
        json!({}),
        json!({"query": "  "}),
        json!({"query": " -- "}),
    ] {
        let err = parse_search_args(&args).unwrap_err();
        assert_eq!(err, "query has no searchable words.");
    }
}

#[test]
fn an_unclosed_quote_still_fails_with_a_tag() {
    let err = parse_search_args(&json!({"query": "\"", "tag": "rule"})).unwrap_err();
    assert_eq!(err, "unclosed quote in query.");
}

#[test]
fn cursors_do_not_cross_between_listing_and_word_search() {
    let entries = five();
    let all = [0, 1, 2, 3, 4];
    let listing = run_tagged(&entries, json!({"tag": "rule", "limit": 2}), &all).unwrap();
    let words = run_tagged(
        &entries,
        json!({"query": "note", "tag": "rule", "limit": 2}),
        &all,
    );
    let words = words.unwrap();
    let mismatch = "cursor does not match this query";
    let listing_cursor = cursor_of(&listing).unwrap();
    let word_args = json!({"query": "note", "tag": "rule", "limit": 2, "cursor": listing_cursor});
    assert_eq!(run_tagged(&entries, word_args, &all).unwrap_err(), mismatch);
    let word_cursor = cursor_of(&words).unwrap();
    let list_args = json!({"tag": "rule", "limit": 2, "cursor": word_cursor});
    assert_eq!(run_tagged(&entries, list_args, &all).unwrap_err(), mismatch);
}

#[test]
fn words_with_a_tag_stay_ranked_and_carry_no_notice() {
    let entries = [user(0, "ship it"), user(1, "other"), user(2, "ship ship")];
    let out = run_tagged(
        &entries,
        json!({"query": "ship", "tag": "rule"}),
        &[0, 1, 2],
    )
    .unwrap();
    assert_eq!(ids(&out), [2, 0]);
    assert!(!out.contains("tags exist only"), "{out}");
    let none = run_tagged(&entries, json!({"query": "absent", "tag": "rule"}), &[0]).unwrap();
    assert_eq!(none, "no hits for absent");
}

#[test]
fn whitespace_and_missing_queries_list_with_a_tag() {
    let entries = five();
    for args in [
        json!({"query": "   ", "tag": "rule"}),
        json!({"tag": "rule"}),
    ] {
        let out = run_tagged(&entries, args, &[1, 3]).unwrap();
        assert_eq!(ids(&out), [3, 1]);
    }
}

#[test]
fn corrupt_entries_are_skipped() {
    let log = format!(
        "{}garbage\n{}",
        written(&user(0, "a")),
        written(&user(2, "c"))
    );
    let mut archive =
        Archive::open(Bytes(log.into_bytes()), None, Bytes(Vec::new()), None).unwrap();
    let request = parse_search_args(&json!({"tag": "rule"}))
        .unwrap()
        .with_tag_ids([0, 1, 2].into_iter().collect());
    let out = search_found(&mut archive, &request, &Marks::new())
        .unwrap()
        .unwrap()
        .text;
    assert_eq!(ids(&out), [2, 0]);
}

#[test]
fn the_listing_snippet_starts_at_the_beginning_of_the_entry() {
    let text = format!("start {}", "filler ".repeat(100));
    let entries = [user(0, &text)];
    let out = run_tagged(&entries, json!({"tag": "rule"}), &[0]).unwrap();
    assert!(out.contains("· start filler"), "{out}");
}
