use super::query::{tokenize, Query};
use super::scan::{CHUNK_ENTRIES, SNIPPET_CHARS};
use super::{parse_search_args, search};
use crate::chat_memory::archive::{Archive, ByteSource};
use crate::chat_memory::{encode_entry, Entry, Role};
use serde_json::{json, Value};
use std::io;

struct Bytes(Vec<u8>);

impl ByteSource for Bytes {
    fn size(&self) -> u64 {
        self.0.len() as u64
    }

    fn read(&mut self, start: u64, end: u64) -> io::Result<Vec<u8>> {
        Ok(self.0[start as usize..end as usize].to_vec())
    }
}

fn at(minute: usize) -> String {
    format!("2026-10-20T12:{:02}:00.000000000+00:00", minute % 60)
}

fn user(minute: usize, text: &str) -> Entry {
    Entry {
        role: Role::User,
        at: at(minute),
        text: text.to_string(),
        corrupt: false,
    }
}

fn timed(role: Role, at: &str, text: &str) -> Entry {
    Entry {
        role,
        at: at.to_string(),
        text: text.to_string(),
        corrupt: false,
    }
}

fn archive(entries: &[Entry]) -> Archive<Bytes> {
    let log: String = entries.iter().map(|e| encode_entry(e).unwrap()).collect();
    Archive::open(Bytes(log.into_bytes()), None, Bytes(Vec::new()), None).unwrap()
}

fn run(entries: &[Entry], args: Value) -> Result<String, String> {
    let request = parse_search_args(&args)?;
    search(&mut archive(entries), &request).unwrap()
}

fn ids(output: &str) -> Vec<usize> {
    output
        .lines()
        .filter_map(|line| line.strip_prefix('#'))
        .filter_map(|rest| rest.split(' ').next()?.parse().ok())
        .collect()
}

fn cursor_of(output: &str) -> Option<String> {
    output
        .lines()
        .find_map(|line| line.strip_prefix("next cursor: "))
        .map(str::to_string)
}

fn words(text: &str) -> Vec<String> {
    tokenize(text).into_iter().map(|t| t.word).collect()
}

#[test]
fn tokenizer_keeps_identifiers_whole_and_lowercases() {
    assert_eq!(
        words("See GUZ-709, foo_bar and foo::bar in a.b. Done-"),
        ["see", "guz-709", "foo_bar", "and", "foo::bar", "in", "a.b", "done"]
    );
    assert_eq!(words("src/main.rs v1.2.3"), ["src", "main.rs", "v1.2.3"]);
    assert!(words("  -- ::  ").is_empty());
}

#[test]
fn tokenizer_records_byte_offsets_of_the_original_text() {
    let tokens = tokenize("é Foo-1");
    assert_eq!(tokens[1].word, "foo-1");
    assert_eq!(tokens[1].start, 3);
}

#[test]
fn query_splits_words_and_phrases_and_merges_repeats() {
    let query = Query::parse("alpha \"Beta gamma\" alpha beta").unwrap();
    assert_eq!(query.normalized(), "alpha \"beta gamma\" beta");
    let merged = Query::parse("beta \"beta\"").unwrap();
    assert!(merged.units[0].required);
    assert!(Query::parse("  \"\" -- ").is_err());
}

#[test]
fn rare_terms_and_more_hits_rank_higher() {
    let entries = [
        user(0, "the cache is fine"),
        user(1, "the cache is fine again"),
        user(2, "zebra cache"),
    ];
    let out = run(&entries, json!({"query": "zebra cache"})).unwrap();
    assert_eq!(ids(&out), [2, 0, 1]);
    let entries = [
        user(0, "cache x y z"),
        user(1, "cache cache cache x"),
        user(2, "w x y z"),
    ];
    let out = run(&entries, json!({"query": "cache"})).unwrap();
    assert_eq!(ids(&out), [1, 0]);
}

#[test]
fn ties_go_to_the_newer_entry() {
    let entries = [user(0, "same words"), user(1, "same words")];
    let out = run(&entries, json!({"query": "same"})).unwrap();
    assert_eq!(ids(&out), [1, 0]);
}

#[test]
fn identifiers_match_whole_tokens_only() {
    let entries = [
        user(0, "only GUZ-7090 landed"),
        user(1, "ship GUZ-709 today"),
        user(2, "the guz-709. shipped"),
    ];
    let out = run(&entries, json!({"query": "GUZ-709"})).unwrap();
    assert_eq!(ids(&out), [2, 1]);
    let out = run(&entries, json!({"query": "guz-7090"})).unwrap();
    assert_eq!(ids(&out), [0]);
    let parts = run(&entries, json!({"query": "guz"})).unwrap();
    assert_eq!(ids(&parts), [2, 1, 0]);
}

#[test]
fn single_words_match_parts_of_joined_tokens() {
    let entries = [
        user(0, "we got rate-limited again"),
        user(1, "a follow-up is due"),
        user(2, "see chat_memory::search for it"),
        user(3, "nothing relevant"),
    ];
    for (query, expected) in [("rate", 0), ("follow", 1), ("search", 2), ("limited", 0)] {
        let out = run(&entries, json!({"query": query})).unwrap();
        assert_eq!(ids(&out), [expected], "{query}");
    }
    let whole = run(&entries, json!({"query": "rate-limited"})).unwrap();
    assert_eq!(ids(&whole), [0]);
    let none = run(&entries, json!({"query": "rate-lim"})).unwrap();
    assert_eq!(none, "no hits for rate-lim");
}

#[test]
fn phrases_match_whole_tokens_not_parts() {
    let entries = [
        user(0, "it is rate-limited"),
        user(1, "the rate limited it"),
    ];
    let out = run(&entries, json!({"query": "\"rate limited\""})).unwrap();
    assert_eq!(ids(&out), [1]);
}

#[test]
fn tokenizer_splits_joined_tokens_into_parts() {
    let tokens = tokenize("a-b::c plain");
    assert_eq!(tokens[0].parts, ["a", "b", "c"]);
    assert!(tokens[1].parts.is_empty());
}

#[test]
fn unclosed_quote_is_an_error() {
    assert_eq!(
        Query::parse("alpha \"beta gamma").unwrap_err(),
        "unclosed quote in query."
    );
    assert!(parse_search_args(&json!({"query": "\"x\" \"y"})).is_err());
    assert!(Query::parse("\"beta gamma\" alpha").is_ok());
}

#[test]
fn since_after_until_is_an_error() {
    let err = parse_search_args(&json!({
        "query": "x", "since": "2026-10-21", "until": "2026-10-20"
    }))
    .unwrap_err();
    assert_eq!(err, "'since' is later than 'until'.");
    let same = json!({"query": "x", "since": "2026-10-20", "until": "2026-10-20"});
    assert!(parse_search_args(&same).is_ok());
    let stamp = json!({
        "query": "x",
        "since": "2026-10-20T12:00:00Z",
        "until": "2026-10-20T12:00:00Z"
    });
    assert!(parse_search_args(&stamp).is_ok());
}

#[test]
fn a_rebuild_mid_search_keeps_new_entries_and_the_cursor_valid() {
    let mut entries: Vec<Entry> = (0..CHUNK_ENTRIES + 44).map(|i| user(i, "hay")).collect();
    let mut archive = archive(&entries);
    let extra = [user(1, "needle"), user(2, "needle")];
    for entry in &extra {
        archive
            .log
            .0
            .extend(encode_entry(entry).unwrap().into_bytes());
    }
    entries.extend(extra);
    archive.log_index.slots[CHUNK_ENTRIES + 10] += 1;
    let request = parse_search_args(&json!({"query": "needle", "limit": 1})).unwrap();
    let out = search(&mut archive, &request).unwrap().unwrap();
    assert_eq!(ids(&out), [entries.len() - 1]);
    let cursor = cursor_of(&out).unwrap();
    assert!(cursor.contains(&format!(".{}.", entries.len())));
    let next = run(
        &entries,
        json!({"query": "needle", "limit": 1, "cursor": cursor}),
    )
    .unwrap();
    assert_eq!(ids(&next), [entries.len() - 2]);
}

#[test]
fn a_quoted_single_word_matches_whole_words_only() {
    let entries = [user(0, "the rate-limited queue"), user(1, "a fixed rate")];
    let bare = run(&entries, json!({"query": "rate"})).unwrap();
    assert_eq!(ids(&bare).len(), 2);
    let quoted = run(&entries, json!({"query": "\"rate\""})).unwrap();
    assert_eq!(ids(&quoted), [1]);
}

#[test]
fn phrases_need_consecutive_tokens_and_are_required() {
    let entries = [
        user(0, "retry the failed job"),
        user(1, "the job failed, then retry"),
        user(2, "a failed job again, failed job"),
    ];
    let scattered = run(&entries, json!({"query": "failed job"})).unwrap();
    assert_eq!(ids(&scattered).len(), 3);
    let phrase = run(&entries, json!({"query": "\"failed job\""})).unwrap();
    assert_eq!(ids(&phrase), [2, 0]);
    let mixed = run(&entries, json!({"query": "\"failed job\" retry"})).unwrap();
    assert_eq!(ids(&mixed), [0, 2]);
}

#[test]
fn role_filter_keeps_one_role() {
    let entries = [
        timed(Role::User, &at(0), "deploy now"),
        timed(Role::Assistant, &at(1), "deploy done"),
        timed(Role::Worker, &at(2), "deploy queued"),
    ];
    let out = run(&entries, json!({"query": "deploy", "role": "worker"})).unwrap();
    assert_eq!(ids(&out), [2]);
    assert!(run(&entries, json!({"query": "deploy", "role": "bot"})).is_err());
}

#[test]
fn since_and_until_accept_timestamps_and_dates() {
    let entries = [
        timed(Role::User, "2026-10-19T23:59:59.500000000+00:00", "note"),
        timed(Role::User, "2026-10-20T12:00:00.000000000+00:00", "note"),
        timed(Role::User, "2026-10-20T23:59:59.999999999+00:00", "note"),
        timed(Role::User, "2026-10-21T00:00:00.000000000+00:00", "note"),
    ];
    let day = json!({"query": "note", "since": "2026-10-20", "until": "2026-10-20"});
    assert_eq!(ids(&run(&entries, day).unwrap()), [2, 1]);
    let stamps = json!({
        "query": "note",
        "since": "2026-10-20T12:00:00Z",
        "until": "2026-10-20T12:00:00+00:00"
    });
    assert_eq!(ids(&run(&entries, stamps).unwrap()), [1]);
    let open = json!({"query": "note", "until": "2026-10-19"});
    assert_eq!(ids(&run(&entries, open).unwrap()), [0]);
    let zoned = json!({"query": "note", "since": "2026-10-20T14:00:00+02:00"});
    assert_eq!(ids(&run(&entries, zoned).unwrap()), [3, 2, 1]);
}

#[test]
fn corrupt_placeholders_keep_their_ids_and_never_match() {
    let log = format!(
        "{}garbage\n{}",
        encode_entry(&user(0, "corrupt needle")).unwrap(),
        encode_entry(&user(2, "needle after")).unwrap()
    );
    let mut archive =
        Archive::open(Bytes(log.into_bytes()), None, Bytes(Vec::new()), None).unwrap();
    let request = parse_search_args(&json!({"query": "corrupt entry needle"})).unwrap();
    let out = search(&mut archive, &request).unwrap().unwrap();
    assert_eq!(ids(&out), [0, 2]);
}

#[test]
fn bad_arguments_are_errors() {
    for args in [
        json!({"query": "x", "since": "yesterday"}),
        json!({"query": "x", "until": "2026-13-40"}),
        json!({"query": "x", "limit": 0}),
        json!({"query": "x", "limit": 51}),
        json!({"query": "x", "limit": "ten"}),
        json!({"query": "x", "cursor": "nope"}),
        json!({"query": "x", "role": 3}),
        json!({"query": "--"}),
        json!({}),
    ] {
        assert!(parse_search_args(&args).is_err(), "{args}");
    }
    let err = parse_search_args(&json!({"query": "x", "since": "yesterday"})).unwrap_err();
    assert!(err.contains("YYYY-MM-DD"));
}

#[test]
fn snippets_are_short_flat_and_centered_on_the_match() {
    let text = format!("{}\n\n needle {}", "pad ".repeat(300), "tail ".repeat(300));
    let out = run(&[user(0, &text)], json!({"query": "needle"})).unwrap();
    let line = out.lines().next().unwrap();
    let snippet = line.split(" · ").nth(3).unwrap();
    assert!(snippet.contains("needle"));
    assert!(snippet.chars().count() <= SNIPPET_CHARS + 2);
    assert!(snippet.starts_with('…') && snippet.ends_with('…'));
    assert!(!snippet.contains('\n'));
}

#[test]
fn hits_show_id_role_time_and_footer() {
    let out = run(&[user(0, "hello world")], json!({"query": "hello"})).unwrap();
    assert_eq!(
        out,
        format!(
            "#0 · user · {} · hello world\n(end of results)\n(read <id> for the full text)",
            at(0)
        )
    );
    assert_eq!(
        run(&[user(0, "hello")], json!({"query": "absent"})).unwrap(),
        "no hits for absent"
    );
}

#[test]
fn cursor_pages_cover_every_hit_once() {
    let entries: Vec<Entry> = (0..45)
        .map(|i| user(i, &format!("alpha {}", "pad ".repeat(i % 7))))
        .collect();
    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    loop {
        let mut args = json!({"query": "alpha", "limit": 10});
        if let Some(c) = &cursor {
            args["cursor"] = json!(c);
        }
        let out = run(&entries, args).unwrap();
        assert!(ids(&out).len() <= 10);
        seen.extend(ids(&out));
        pages += 1;
        cursor = cursor_of(&out);
        if cursor.is_none() {
            assert!(out.contains("(end of results)"));
            break;
        }
    }
    assert_eq!(pages, 5);
    let mut sorted = seen.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted, (0..45).collect::<Vec<_>>());
    let first = run(&entries, json!({"query": "alpha", "limit": 45})).unwrap();
    assert_eq!(ids(&first), seen);
}

#[test]
fn cursor_is_rejected_after_a_query_or_filter_change() {
    let entries: Vec<Entry> = (0..30).map(|i| user(i, "alpha beta")).collect();
    let out = run(&entries, json!({"query": "alpha", "limit": 5})).unwrap();
    let cursor = cursor_of(&out).unwrap();
    let with = |extra: Value| {
        let mut args = json!({"query": "alpha", "limit": 5, "cursor": cursor});
        for (k, v) in extra.as_object().unwrap() {
            args[k] = v.clone();
        }
        run(&entries, args)
    };
    assert!(with(json!({})).is_ok());
    for change in [
        json!({"query": "beta"}),
        json!({"role": "user"}),
        json!({"since": "2026-10-20"}),
        json!({"until": "2026-10-21"}),
        json!({"limit": 6}),
    ] {
        assert_eq!(
            with(change.clone()).unwrap_err(),
            "cursor does not match this query",
            "{change}"
        );
    }
    assert!(with(json!({"query": "ALPHA"})).is_ok());
}

#[test]
fn cursor_is_stale_after_the_log_grows() {
    let mut entries: Vec<Entry> = (0..30).map(|i| user(i, "alpha")).collect();
    let out = run(&entries, json!({"query": "alpha", "limit": 5})).unwrap();
    let cursor = cursor_of(&out).unwrap();
    entries.push(user(31, "alpha"));
    let err = run(
        &entries,
        json!({"query": "alpha", "limit": 5, "cursor": cursor}),
    );
    assert_eq!(err.unwrap_err(), "stale cursor; the log changed");
}

#[test]
fn a_cursor_past_the_last_hit_ends_the_results() {
    let entries = [user(0, "alpha"), user(1, "alpha")];
    let out = run(&entries, json!({"query": "alpha", "limit": 1})).unwrap();
    let cursor = cursor_of(&out).unwrap();
    let last = run(
        &entries,
        json!({"query": "alpha", "limit": 1, "cursor": cursor}),
    )
    .unwrap();
    assert_eq!(ids(&last), [0]);
    assert!(last.contains("(end of results)"));
}

#[test]
fn scanning_crosses_chunk_boundaries() {
    let entries: Vec<Entry> = (0..CHUNK_ENTRIES * 2 + 3)
        .map(|i| user(i, if i % 100 == 0 { "needle" } else { "hay" }))
        .collect();
    let out = run(&entries, json!({"query": "needle", "limit": 50})).unwrap();
    assert_eq!(ids(&out), [500, 400, 300, 200, 100, 0]);
}

#[test]
fn empty_log_has_no_hits() {
    assert_eq!(run(&[], json!({"query": "x"})).unwrap(), "no hits for x");
}

const CORPUS: [&str; 12] = [
    "We decided to use SQLite for the chat index instead of RocksDB.",
    "The deploy to staging failed because the AWS_PROFILE variable was unset.",
    "Please rename the config flag --max-turns to --turn-limit before release.",
    "GUZ-709 added log.idx so read and zoom load only the entries they show.",
    "GUZ-7090 is an unrelated ticket about billing exports.",
    "The nap pass compresses pairs of messages into summaries at 512 bytes.",
    "Release 0.3.0 is blocked on the clippy too_many_arguments warning.",
    "Lunch plans: ramen near the office on Thursday.",
    "The watch command ran cargo test and exited with code 101.",
    "Memory tree blocks are aligned power-of-two ranges like 16-31.",
    "Use foo::bar::baz for the helper and keep foo_bar private.",
    "Worker B reported the merge conflict in tree.rs and asked for guidance.",
];

#[test]
fn evidence_recall_at_5_is_perfect_on_the_labeled_corpus() {
    let entries: Vec<Entry> = CORPUS.iter().enumerate().map(|(i, t)| user(i, t)).collect();
    let labeled: [(&str, usize); 10] = [
        ("which database did we choose for the index", 0),
        ("deploy staging AWS_PROFILE", 1),
        ("\"turn-limit\" flag rename", 2),
        ("GUZ-709", 3),
        ("nap summaries 512 bytes", 5),
        ("clippy warning release blocked", 6),
        ("watch exited code 101", 8),
        ("power-of-two aligned ranges", 9),
        ("foo::bar::baz helper", 10),
        ("merge conflict tree.rs", 11),
    ];
    let hits = labeled
        .iter()
        .filter(|(query, expected)| {
            let out = run(&entries, json!({"query": query, "limit": 5})).unwrap();
            ids(&out).contains(expected)
        })
        .count();
    assert_eq!(hits, labeled.len(), "recall@5 = {hits}/{}", labeled.len());
}
