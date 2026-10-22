use super::*;
use crate::wire::provider::anthropic::Anthropic;
use crate::wire::provider::RequestFacts;
use crate::wire::usage::CacheWrite;
use chrono::{DateTime, TimeZone, Utc};
use serde_json::json;

fn t(secs: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
}

fn blk(text: &str, ttl: Option<&str>) -> Value {
    match ttl {
        Some(ttl) => {
            json!({"type": "text", "text": text, "cache_control": {"type": "ephemeral", "ttl": ttl}})
        }
        None => json!({"type": "text", "text": text}),
    }
}

fn usage(uncached: u64, read: u64) -> UsageOutcome {
    UsageOutcome::Reported(Usage {
        input_uncached: uncached,
        cache_read: read,
        cache_writes: vec![],
        output: 1,
        reasoning: None,
        other: vec![],
    })
}

fn call(seq: u64, start: i64, end: i64, blocks: Vec<Value>, usage: UsageOutcome) -> CallRecord {
    CallRecord {
        seq,
        started_at: t(start),
        first_byte_at: None,
        ended_at: t(end),
        method: "POST".into(),
        path: "/v1/messages".into(),
        status: Some(200),
        kind: CallKind::Inference,
        facts: RequestFacts {
            model: Some("m".into()),
            params: serde_json::Map::new(),
        },
        request_headers: vec![],
        response_headers: vec![],
        request_body: RequestBody::Json(json!({"messages": [{"role": "user", "content": blocks}]})),
        raw_usage: None,
        usage,
        input_equivalent: None,
        complete: true,
        error: None,
        response_events: None,
    }
}

fn pair(a_tail: &str, b_tail: &str, b_usage: UsageOutcome) -> (CallRecord, CallRecord) {
    let head = || blk("shared head", Some("5m"));
    (
        call(1, 0, 10, vec![head(), blk(a_tail, None)], usage(5, 0)),
        call(2, 20, 30, vec![head(), blk(b_tail, None)], b_usage),
    )
}

fn view_of(a: &CallRecord, b: &CallRecord, session: &[CallRecord]) -> DiffView {
    diff_view(&DiffInput {
        a: Some(a),
        b,
        session,
        provider: &Anthropic,
        base_picked: false,
    })
}

fn verdict_of(a: &CallRecord, b: &CallRecord) -> Verdict {
    view_of(a, b, &[a.clone(), b.clone()]).explanation.verdict
}

#[test]
fn pick_base_prefers_longest_prefix_over_predecessor() {
    let x = |tail: &str| vec![blk("x1", None), blk("x2", None), blk(tail, None)];
    let mut other = call(2, 5, 6, vec![blk("y1", None)], usage(1, 0));
    other.facts.model = Some("other".into());
    let calls = vec![
        call(1, 0, 1, x("x3"), usage(1, 0)),
        other,
        call(3, 10, 11, x("x4"), usage(1, 0)),
    ];
    assert_eq!(
        pick_base(&calls, 3, "s", &Anthropic).unwrap().unwrap().seq,
        1
    );
}

#[test]
fn pick_base_tie_takes_highest_seq() {
    let body = || vec![blk("x1", None)];
    let calls = vec![
        call(1, 0, 1, body(), usage(1, 0)),
        call(2, 2, 3, body(), usage(1, 0)),
        call(3, 4, 5, body(), usage(1, 0)),
    ];
    assert_eq!(
        pick_base(&calls, 3, "s", &Anthropic).unwrap().unwrap().seq,
        2
    );
}

#[test]
fn pick_base_without_shared_prefix_is_none() {
    let calls = vec![
        call(1, 0, 1, vec![blk("p", None)], usage(1, 0)),
        call(2, 2, 3, vec![blk("q", None)], usage(1, 0)),
    ];
    assert_eq!(pick_base(&calls, 2, "s", &Anthropic).unwrap(), None);
    assert_eq!(
        pick_base(&calls, 9, "s", &Anthropic)
            .unwrap_err()
            .to_string(),
        "no call 9 in session s"
    );
}

#[test]
fn facts_and_cache_headers_differ_case_insensitively() {
    let (mut a, mut b) = pair("t", "t", usage(1, 0));
    b.facts.model = Some("n".into());
    a.facts.params.insert("max_tokens".into(), json!(1));
    b.facts.params.insert("max_tokens".into(), json!(2));
    b.facts.params.insert("thinking".into(), json!({"a": 1}));
    a.request_headers = vec![
        ("Anthropic-Beta".into(), "x".into()),
        ("anthropic-version".into(), "1".into()),
        ("user-agent".into(), "p".into()),
    ];
    b.request_headers = vec![
        ("anthropic-beta".into(), "x".into()),
        ("Anthropic-Version".into(), "2".into()),
        ("user-agent".into(), "q".into()),
    ];
    let view = view_of(&a, &b, &[]);
    let names: Vec<&str> = view.facts.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["model", "max_tokens", "thinking"]);
    assert_eq!(view.facts[2].a, None);
    assert_eq!(
        view.headers,
        [HeaderDiff {
            name: "anthropic-version".into(),
            a: Some("1".into()),
            b: Some("2".into())
        }]
    );
}

#[test]
fn divergence_reports_offset_excerpt_and_removed_added() {
    let (mut a, b) = pair("same prefix then A", "same prefix then B", usage(1, 0));
    if let RequestBody::Json(body) = &mut a.request_body {
        body["messages"][0]["content"]
            .as_array_mut()
            .unwrap()
            .push(blk("tail", None));
    }
    let view = view_of(&a, &b, &[]);
    let div = view.divergence.unwrap();
    assert_eq!(view.shared_segments, 1);
    assert_eq!(div.index, 1);
    assert_eq!(div.label_a, "msg[0].1 user/text");
    let text = segments_of(&b, &Anthropic).unwrap()[1].text.clone();
    assert_eq!(&text[div.offset..div.offset + 1], "B");
    assert_eq!(div.b_excerpt, text[div.offset..]);
    assert!(div.a_excerpt.starts_with('A'));
    assert_eq!(view.removed, ["msg[0].2 user/text"]);
    assert!(view.added.is_empty());
}

#[test]
fn excerpt_snaps_to_char_boundaries_and_caps_length() {
    let long = "z".repeat(100);
    let (a, b) = pair(
        &format!("\u{e9}{long}"),
        &format!("\u{eb}{long}"),
        usage(1, 0),
    );
    let div = view_of(&a, &b, &[]).divergence.unwrap();
    assert!(div.a_excerpt.starts_with('\u{e9}'));
    assert!(div.b_excerpt.starts_with('\u{eb}'));
    assert!(div.a_excerpt.len() <= EXCERPT_BYTES);
    assert!(excerpt("\u{e9}\u{e9}\u{e9}", 3).len() <= EXCERPT_BYTES);
}

#[test]
fn b_extends_a_has_no_divergence_and_only_added() {
    let head = || blk("h", Some("5m"));
    let a = call(1, 0, 10, vec![head()], usage(1, 0));
    let b = call(2, 20, 30, vec![head(), blk("more", None)], usage(0, 100));
    let view = view_of(&a, &b, &[]);
    assert_eq!(view.divergence, None);
    assert_eq!(view.shared_segments, 1);
    assert!(view.removed.is_empty());
    assert_eq!(view.added, ["msg[0].1 user/text"]);
    assert_eq!(
        view.explanation.verdict,
        Verdict::PrefixShared {
            relation: Relation::Extends
        }
    );
}

#[test]
fn string_content_shares_segment_with_block_content() {
    let mut a = call(1, 0, 10, vec![], usage(1, 0));
    a.request_body = RequestBody::Json(
        json!({"messages": [{"role": "user", "content": "same"}, {"role": "user", "content": "x"}]}),
    );
    let b = call(
        2,
        20,
        30,
        vec![blk("same", None), blk("y", None)],
        usage(1, 0),
    );
    let b_text = segments_of(&b, &Anthropic).unwrap();
    assert_eq!(segments_of(&a, &Anthropic).unwrap()[0], b_text[0]);
    assert_eq!(view_of(&a, &b, &[]).shared_segments, 1);
}

#[test]
fn no_shared_breakpoint() {
    let a = call(1, 0, 10, vec![blk("p", None)], usage(1, 0));
    let b = call(2, 20, 30, vec![blk("p", None)], usage(1, 7));
    let explanation = view_of(&a, &b, &[]).explanation;
    assert_eq!(explanation.verdict, Verdict::NoSharedBreakpoint);
    assert_eq!(explanation.cache_read, Some(7));
    assert_eq!(explanation.servable_bytes, 0);
}

#[test]
fn cache_agreeing_with_estimate_means_prefix_changed() {
    let (a, b) = pair("a", "b", usage(0, 100));
    let view = view_of(&a, &b, &[]);
    assert_eq!(
        view.explanation.verdict,
        Verdict::PrefixChanged {
            label: "msg[0].1 user/text".into()
        }
    );
    assert!(view.explanation.servable_tokens.unwrap() > 0);
}

#[test]
fn model_header_param_verdicts_in_order() {
    let (mut a, mut b) = pair("a", "b", written(0, 100));
    a.facts.params.insert("thinking".into(), json!(1));
    assert_eq!(
        verdict_of(&a, &b),
        Verdict::ParamChanged {
            names: vec!["thinking".into()]
        }
    );
    b.request_headers = vec![("anthropic-beta".into(), "x".into())];
    assert_eq!(
        verdict_of(&a, &b),
        Verdict::HeaderChanged {
            names: vec!["anthropic-beta".into()]
        }
    );
    b.facts.model = Some("n".into());
    assert_eq!(
        verdict_of(&a, &b),
        Verdict::ModelChanged {
            a: Some("m".into()),
            b: Some("n".into())
        }
    );
}

#[test]
fn irrelevant_param_does_not_explain() {
    let (mut a, b) = pair("a", "b", written(0, 100));
    a.facts.params.insert("max_tokens".into(), json!(1));
    assert!(matches!(verdict_of(&a, &b), Verdict::Unexplained { .. }));
}

#[test]
fn ttl_gap_beyond_ttl_expires() {
    let (a, mut b) = pair("a", "b", written(0, 100));
    b.started_at = t(400);
    assert_eq!(
        verdict_of(&a, &b),
        Verdict::TtlExpired {
            gap_secs: 390,
            ttl_secs: 300,
            since_seq: 1
        }
    );
}

#[test]
fn intermediate_refresh_prevents_ttl_expiry() {
    let (a, mut b) = pair("a", "b", written(0, 100));
    b.seq = 3;
    b.started_at = t(400);
    let refresh = call(
        2,
        200,
        350,
        vec![blk("shared head", Some("5m")), blk("mid", None)],
        usage(1, 1),
    );
    let session = [a.clone(), refresh, b.clone()];
    assert!(matches!(
        view_of(&a, &b, &session).explanation.verdict,
        Verdict::Unexplained { .. }
    ));
}

#[test]
fn intermediate_without_the_breakpoint_does_not_refresh() {
    let (a, mut b) = pair("a", "b", written(0, 100));
    b.seq = 3;
    b.started_at = t(400);
    let no_marker = call(
        2,
        200,
        350,
        vec![blk("shared head", None), blk("mid", None)],
        usage(1, 1),
    );
    let session = [a.clone(), no_marker, b.clone()];
    assert_eq!(
        view_of(&a, &b, &session).explanation.verdict,
        Verdict::TtlExpired {
            gap_secs: 390,
            ttl_secs: 300,
            since_seq: 1
        }
    );
}

#[test]
fn intermediate_with_a_later_breakpoint_refreshes() {
    let (a, mut b) = pair("a", "b", written(0, 100));
    b.seq = 3;
    b.started_at = t(400);
    let later = call(
        2,
        200,
        350,
        vec![blk("shared head", Some("5m")), blk("mid", Some("5m"))],
        usage(1, 1),
    );
    let session = [a.clone(), later, b.clone()];
    assert!(matches!(
        view_of(&a, &b, &session).explanation.verdict,
        Verdict::Unexplained { .. }
    ));
}

fn written(read: u64, write: u64) -> UsageOutcome {
    UsageOutcome::Reported(Usage {
        input_uncached: 0,
        cache_read: read,
        cache_writes: vec![CacheWrite {
            tier: CacheTier::Unspecified,
            tokens: write,
        }],
        output: 1,
        reasoning: None,
        other: vec![],
    })
}

fn two_bp(seq: u64, start: i64, tail: &str, usage: UsageOutcome) -> CallRecord {
    call(
        seq,
        start,
        start + 10,
        vec![blk("shared head", Some("5m")), blk(tail, Some("5m"))],
        usage,
    )
}

#[test]
fn anchor_at_a_gives_exact_tokens() {
    let a = call(
        1,
        0,
        10,
        vec![blk("shared head", Some("5m")), blk("a", None)],
        written(20, 80),
    );
    let b = two_bp(2, 20, "b", usage(5, 0));
    let explanation = view_of(&a, &b, &[a.clone(), b.clone()]).explanation;
    assert_eq!(explanation.servable_tokens, Some(100));
    assert_eq!(explanation.method, Some(TokenMethod::Anchor { seq: 1 }));
    assert!(matches!(explanation.verdict, Verdict::Unexplained { .. }));
}

#[test]
fn anchor_at_b_itself() {
    let (a, b) = pair("a", "b", written(30, 70));
    let explanation = view_of(&a, &b, &[a.clone(), b.clone()]).explanation;
    assert_eq!(explanation.servable_tokens, Some(100));
    assert_eq!(explanation.method, Some(TokenMethod::Anchor { seq: 2 }));
}

#[test]
fn anchor_ignored_when_its_last_breakpoint_is_past_the_shared_one() {
    let a = two_bp(1, 0, "a", written(500, 500));
    let (_, b) = pair("a", "b", written(30, 70));
    let explanation = view_of(&a, &b, &[a.clone(), b.clone()]).explanation;
    assert_eq!(explanation.method, Some(TokenMethod::Anchor { seq: 2 }));
    assert_eq!(explanation.servable_tokens, Some(100));
}

#[test]
fn no_anchor_falls_back_to_bytes() {
    let (a, b) = pair("a", "b", usage(0, 100));
    let explanation = view_of(&a, &b, &[]).explanation;
    assert_eq!(explanation.method, Some(TokenMethod::Bytes));
}

#[test]
fn anchor_says_miss_where_byte_estimate_says_agree() {
    let a = call(
        1,
        0,
        10,
        vec![blk("shared head", Some("5m")), blk("a", None)],
        usage(0, 1000),
    );
    let b = two_bp(2, 20, "b", usage(0, 100));
    let by_bytes = view_of(&a, &b, &[]).explanation.verdict;
    assert!(matches!(by_bytes, Verdict::PrefixChanged { .. }));
    let anchored = view_of(&a, &b, &[a.clone(), b.clone()]).explanation;
    assert_eq!(anchored.servable_tokens, Some(1000));
    assert!(matches!(anchored.verdict, Verdict::Unexplained { .. }));
}

fn no_base(b: &CallRecord) -> DiffView {
    diff_view(&DiffInput {
        a: None,
        b,
        session: &[],
        provider: &Anthropic,
        base_picked: true,
    })
}

#[test]
fn no_base_is_a_cold_write_or_an_outside_cache_read() {
    let (_, cold) = pair("a", "b", written(0, 100));
    let view = no_base(&cold);
    assert_eq!(view.a, None);
    assert_eq!(view.explanation.verdict, Verdict::NoBase);
    let table = diff_table(&view);
    assert!(
        table.starts_with("diff - -> 2 (no earlier call shares a prompt prefix)\n"),
        "{table}"
    );
    assert!(
        table.contains("cache: cold write: no earlier call in this session shares a prompt prefix"),
        "{table}"
    );
    let (_, warm) = pair("a", "b", usage(5, 40));
    let table = diff_table(&no_base(&warm));
    assert!(
        table.contains("the cache read came from outside this session"),
        "{table}"
    );
    let json = serde_json::to_value(no_base(&warm)).unwrap();
    assert_eq!(json["explanation"]["verdict"]["kind"], "no_base");
    assert_eq!(json["a"], Value::Null);
    assert_eq!(json["explanation"]["cache_read"], 40);
}

#[test]
fn no_base_with_non_json_target_is_no_segments() {
    let (_, mut b) = pair("a", "b", usage(1, 0));
    b.request_body = RequestBody::Base64(String::new());
    assert_eq!(no_base(&b).explanation.verdict, Verdict::NoSegments);
}

#[test]
fn small_gap_is_unexplained_with_invisible_causes() {
    let (a, b) = pair("a", "b", written(0, 100));
    match verdict_of(&a, &b) {
        Verdict::Unexplained { invisible } => assert_eq!(invisible.len(), 3),
        other => panic!("{other:?}"),
    }
}

#[test]
fn missing_usage_and_non_json_bodies() {
    let (a, mut b) = pair("a", "b", UsageOutcome::Missing);
    let explanation = view_of(&a, &b, &[]).explanation;
    assert_eq!(explanation.verdict, Verdict::UsageMissing);
    assert_eq!(
        (explanation.servable_tokens, explanation.cache_read),
        (None, None)
    );
    b.request_body = RequestBody::Base64(String::new());
    let view = view_of(&a, &b, &[]);
    assert_eq!(view.explanation.verdict, Verdict::NoSegments);
    assert_eq!(view.shared_segments, 0);
}

#[test]
fn partial_usage_counts_like_reported() {
    let (a, reported) = pair("a", "b", usage(5, 20));
    let mut partial = reported.clone();
    if let UsageOutcome::Reported(u) = reported.usage.clone() {
        partial.usage = UsageOutcome::Partial(u);
    }
    assert_eq!(
        view_of(&a, &reported, &[]).explanation,
        view_of(&a, &partial, &[]).explanation
    );
}

#[test]
fn cache_writes_count_toward_prompt_tokens() {
    let (a, mut b) = pair("a", "b", usage(0, 0));
    b.usage = UsageOutcome::Reported(Usage {
        input_uncached: 0,
        cache_read: 0,
        cache_writes: vec![CacheWrite {
            tier: CacheTier::Unspecified,
            tokens: 100,
        }],
        output: 1,
        reasoning: None,
        other: vec![],
    });
    assert!(view_of(&a, &b, &[]).explanation.servable_tokens.unwrap() > 0);
}

#[test]
fn table_names_verdict_and_escapes_excerpts() {
    let (a, mut b) = pair("a\nb", "c", usage(5, 0));
    b.started_at = t(400);
    let table = diff_table(&view_of(&a, &b, &[]));
    assert!(table.starts_with("diff 1 -> 2\n"), "{table}");
    assert!(table.contains("shared segments: 1"), "{table}");
    assert!(table.contains("\\n"), "{table}");
    assert!(table.contains("cache: ttl expired"), "{table}");
    assert!(table.contains("agree threshold 90%"), "{table}");
    let none = Explanation {
        servable_tokens: None,
        method: None,
        cache_read: None,
        ..view_of(&a, &b, &[]).explanation
    };
    let bare = DiffView {
        explanation: none,
        ..view_of(&a, &b, &[])
    };
    assert!(diff_table(&bare).ends_with("(cache_read -; agree threshold 90%)"));
}

#[test]
fn moved_breakpoint_is_not_a_divergence() {
    let a = call(
        1,
        0,
        10,
        vec![blk("h", Some("5m")), blk("q1", Some("5m"))],
        usage(1, 0),
    );
    let b = call(
        2,
        20,
        30,
        vec![
            blk("h", Some("5m")),
            blk("q1", None),
            blk("q2", None),
            blk("q3", Some("5m")),
        ],
        usage(0, 100),
    );
    let view = view_of(&a, &b, &[]);
    assert_eq!(view.shared_segments, 2);
    assert_eq!(view.divergence, None);
    assert!(view.removed.is_empty());
    assert_eq!(view.added, ["msg[0].2 user/text", "msg[0].3 user/text"]);
    assert_eq!(
        view.explanation.verdict,
        Verdict::PrefixShared {
            relation: Relation::Extends
        }
    );
    let table = diff_table(&view);
    assert!(table.contains("b extends a"), "{table}");
}

#[test]
fn marker_only_difference_does_not_count_for_pick_base() {
    let target = || vec![blk("x1", None), blk("x2", None), blk("x3", None)];
    let calls = vec![
        call(
            1,
            0,
            1,
            vec![
                blk("x1", Some("5m")),
                blk("x2", Some("5m")),
                blk("x3", None),
            ],
            usage(1, 0),
        ),
        call(2, 2, 3, vec![blk("x1", None), blk("z", None)], usage(1, 0)),
        call(3, 4, 5, target(), usage(1, 0)),
    ];
    assert_eq!(
        pick_base(&calls, 3, "s", &Anthropic).unwrap().unwrap().seq,
        1
    );
}

#[test]
fn divergence_excerpts_exclude_the_marker() {
    let head = || blk("h", Some("5m"));
    let a = call(1, 0, 10, vec![head(), blk("AAA", Some("5m"))], usage(1, 0));
    let b = call(2, 20, 30, vec![head(), blk("BBB", None)], usage(1, 0));
    let div = view_of(&a, &b, &[]).divergence.unwrap();
    assert!(!div.a_excerpt.contains("cache_control"), "{div:?}");
    assert!(!div.b_excerpt.contains("cache_control"), "{div:?}");
    assert!(div.a_excerpt.contains("AAA"));
    assert!(div.b_excerpt.contains("BBB"));
}

#[test]
fn b_prefix_of_a_and_identical_prompts_say_so() {
    let head = || blk("h", Some("5m"));
    let long = call(1, 0, 10, vec![head(), blk("more", None)], usage(1, 0));
    let short = call(2, 20, 30, vec![head()], usage(0, 100));
    let view = view_of(&long, &short, &[]);
    assert_eq!(
        view.explanation.verdict,
        Verdict::PrefixShared {
            relation: Relation::PrefixOf
        }
    );
    let table = diff_table(&view);
    assert!(
        table.contains("cache served the shared prefix; b is a prefix of a"),
        "{table}"
    );
    let same = call(3, 40, 50, vec![head()], usage(0, 100));
    let view = view_of(&short, &same, &[]);
    assert_eq!(
        view.explanation.verdict,
        Verdict::PrefixShared {
            relation: Relation::Identical
        }
    );
    let table = diff_table(&view);
    assert!(
        table.contains("cache served the shared prefix; prompts are identical"),
        "{table}"
    );
}

#[test]
fn relation_compares_lengths_against_the_shared_count() {
    assert_eq!(relation(1, 2, 1), Relation::Extends);
    assert_eq!(relation(2, 1, 1), Relation::PrefixOf);
    assert_eq!(relation(1, 1, 1), Relation::Identical);
    assert_eq!(relation(0, 0, 0), Relation::Identical);
}

#[test]
fn cache_served_verdicts_serialize_without_null_labels() {
    let shared = |relation| serde_json::to_value(Verdict::PrefixShared { relation }).unwrap();
    assert_eq!(
        shared(Relation::Extends),
        json!({"kind": "prefix_shared", "relation": "extends"})
    );
    assert_eq!(
        shared(Relation::PrefixOf),
        json!({"kind": "prefix_shared", "relation": "prefix_of"})
    );
    assert_eq!(
        shared(Relation::Identical),
        json!({"kind": "prefix_shared", "relation": "identical"})
    );
    let (a, b) = pair("a", "b", usage(0, 100));
    let verdict = serde_json::to_value(view_of(&a, &b, &[]).explanation.verdict).unwrap();
    assert_eq!(
        verdict,
        json!({"kind": "prefix_changed", "label": "msg[0].1 user/text"})
    );
}

#[test]
fn check_pair_rejects_a_not_before_b() {
    let (a, b) = pair("a", "b", usage(1, 0));
    assert_eq!(check_pair(Some(&a), &b), Ok(()));
    assert_eq!(check_pair(None, &b), Ok(()));
    let message = check_pair(Some(&b), &a).unwrap_err().to_string();
    assert_eq!(message, "call A must come before call B (got 2 -> 1)");
    let message = check_pair(Some(&a), &a).unwrap_err().to_string();
    assert_eq!(message, "call A must come before call B (got 1 -> 1)");
}

#[test]
fn check_pair_rejects_non_inference() {
    let (a, b) = pair("a", "b", usage(1, 0));
    let mut other = b.clone();
    other.kind = CallKind::Other;
    assert_eq!(
        check_pair(Some(&a), &other).unwrap_err().to_string(),
        "call 2 is not an inference call"
    );
    let mut other_a = a.clone();
    other_a.kind = CallKind::Other;
    assert_eq!(
        check_pair(Some(&other_a), &b).unwrap_err().to_string(),
        "call 1 is not an inference call"
    );
}

#[test]
fn header_for_non_json_target_does_not_claim_a_pick() {
    let (_, mut b) = pair("a", "b", usage(1, 0));
    b.request_body = RequestBody::Base64(String::new());
    let table = diff_table(&no_base(&b));
    assert!(
        table.starts_with("diff - -> 2 (request body is not JSON; no base picked)"),
        "{table}"
    );
}

#[test]
fn non_json_a_keeps_b_cache_read() {
    let (mut a, b) = pair("a", "b", usage(5, 40));
    a.request_body = RequestBody::Base64(String::new());
    let view = view_of(&a, &b, &[]);
    assert_eq!(view.explanation.cache_read, Some(40));
    assert_eq!(view.explanation.verdict, Verdict::NoSegments);
}

#[test]
fn byte_estimate_scales_servable_bytes_by_prompt_tokens() {
    let (a, b) = pair("a", "b", usage(50, 100));
    let explanation = view_of(&a, &b, &[]).explanation;
    let segments = segments_of(&b, &Anthropic).unwrap();
    let total: usize = segments.iter().map(|s| s.text.len()).sum();
    assert_eq!(explanation.method, Some(TokenMethod::Bytes));
    assert_eq!(explanation.servable_bytes, segments[0].text.len());
    assert_eq!(
        explanation.servable_tokens,
        Some((explanation.servable_bytes as f64 * 150.0 / total as f64).round() as u64)
    );
    assert!(explanation.servable_tokens.unwrap() < 150);
    assert!(explanation.servable_tokens.unwrap() > 50);
}

#[test]
fn table_lists_fact_changes_with_dash_for_absent_values() {
    let (mut a, mut b) = pair("t", "t", usage(1, 0));
    b.facts.model = Some("n".into());
    a.facts.params.insert("max_tokens".into(), json!(1));
    let table = diff_table(&view_of(&a, &b, &[]));
    assert!(table.contains("fact model: \"m\" -> \"n\"\n"), "{table}");
    assert!(table.contains("fact max_tokens: 1 -> -\n"), "{table}");
}

#[test]
fn table_omits_removed_and_added_when_empty() {
    let (a, b) = pair("t", "t", usage(1, 0));
    let table = diff_table(&view_of(&a, &b, &[]));
    assert!(!table.contains("removed:"), "{table}");
    assert!(!table.contains("added:"), "{table}");
}

#[test]
fn ttl_gap_equal_to_ttl_does_not_expire() {
    let (a, mut b) = pair("a", "b", written(0, 100));
    b.started_at = t(310);
    assert!(matches!(verdict_of(&a, &b), Verdict::Unexplained { .. }));
    b.started_at = t(311);
    assert!(matches!(verdict_of(&a, &b), Verdict::TtlExpired { .. }));
}

#[test]
fn anchor_requires_identical_prefix_content() {
    let (a, b) = pair("a", "b", written(30, 70));
    let other = call(
        1,
        0,
        10,
        vec![blk("other head", Some("5m"))],
        written(500, 500),
    );
    let explanation = view_of(&a, &b, &[other]).explanation;
    assert_eq!(explanation.method, Some(TokenMethod::Bytes));
}

#[test]
fn anchor_ignores_non_inference_calls() {
    let (a, b) = pair("a", "b", written(30, 70));
    let mut other = call(
        1,
        0,
        10,
        vec![blk("shared head", Some("5m")), blk("a", None)],
        written(500, 500),
    );
    other.kind = CallKind::Other;
    let explanation = view_of(&a, &b, &[other]).explanation;
    assert_eq!(explanation.method, Some(TokenMethod::Bytes));
}

#[test]
fn calls_without_segments_are_skipped_by_anchor_and_refresh() {
    let (a, mut b) = pair("a", "b", written(0, 100));
    b.seq = 3;
    b.started_at = t(400);
    let empty = call(2, 200, 350, vec![], usage(1, 1));
    let session = [a.clone(), empty, b.clone()];
    assert_eq!(
        view_of(&a, &b, &session).explanation.verdict,
        Verdict::TtlExpired {
            gap_secs: 390,
            ttl_secs: 300,
            since_seq: 1
        }
    );
}
