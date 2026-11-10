use super::*;
use crate::chat_memory::fixtures::texted;
use crate::chat_memory::Entry;
use crate::retry::FailureKind;

const MODEL: &str = "claude-haiku-5-5";

fn flags(rule: bool, approval: bool) -> Flags {
    Flags { rule, approval }
}

fn state(text: &str) -> TagState {
    decode_rows(text.as_bytes())
}

fn row(n: usize, kind: &RowKind) -> String {
    encode_row(&TagRow {
        n,
        kind: kind.clone(),
        model: MODEL.to_string(),
        prompt: PROMPT_VERSION.to_string(),
        at: "2026-11-09T00:00:00Z".to_string(),
    })
    .unwrap()
}

#[test]
fn start_row_is_the_first_row_and_decodes() {
    let line = encode_start(7, "2026-11-09T00:00:00Z").unwrap();
    assert_eq!(
        line,
        "{\"v\":1,\"start\":7,\"at\":\"2026-11-09T00:00:00Z\"}\n"
    );
    assert_eq!(state(&line).start, Some(7));
}

#[test]
fn tagged_row_round_trips() {
    let line = row(3, &RowKind::Tagged(flags(true, true)));
    assert!(line.ends_with('\n'));
    let value: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(value["v"], 1);
    assert_eq!(value["n"], 3);
    assert_eq!(value["rule"], true);
    assert_eq!(value["approval"], true);
    assert_eq!(value["model"], MODEL);
    assert_eq!(value["prompt"], PROMPT_VERSION);
    assert_eq!(state(&line).tags.get(&3).copied(), Some(flags(true, true)));
}

#[test]
fn failed_and_skipped_rows_carry_their_marker_and_no_flags() {
    let failed = row(4, &RowKind::Failed);
    let skipped = row(5, &RowKind::Skipped);
    assert!(failed.contains("\"failed\":true") && !failed.contains("\"rule\""));
    assert!(skipped.contains("\"skipped\":true") && !skipped.contains("\"rule\""));
    let both = state(&format!("{failed}{skipped}"));
    assert_eq!(both.failures.get(&4), Some(&1));
    assert!(both.skipped.contains(&5));
    assert!(both.tags.is_empty());
}

#[test]
fn torn_and_unknown_lines_are_ignored_and_the_last_tag_wins() {
    let text = format!(
        "{}{}{}garbage\n{{\"v\":1,\"n\":9,\"rule\":tr",
        encode_start(0, "t").unwrap(),
        row(1, &RowKind::Tagged(flags(false, false))),
        row(1, &RowKind::Tagged(flags(false, true))),
    );
    let s = state(&text);
    assert_eq!(s.tags.get(&1).copied(), Some(flags(false, true)));
    assert_eq!(s.tags.len(), 1);
}

#[test]
fn untagged_covers_start_to_end_and_skips_done_and_corrupt() {
    let mut entries = texted(8, |i| format!("m{i}"));
    entries[6] = Entry::corrupt();
    let s = state(&format!(
        "{}{}{}",
        encode_start(2, "t").unwrap(),
        row(3, &RowKind::Tagged(flags(false, false))),
        row(4, &RowKind::Skipped),
    ));
    assert_eq!(untagged(&entries, &s), vec![2, 5, 7]);
}

#[test]
fn without_a_start_row_nothing_is_untagged() {
    let entries = texted(3, |i| format!("m{i}"));
    assert!(untagged(&entries, &TagState::default()).is_empty());
}

#[test]
fn batch_lists_targets_with_three_context_entries_and_cuts_text() {
    let mut entries = texted(6, |i| format!("m{i}"));
    entries[5].text = "x".repeat(2000);
    let batch = build_batch(&entries, &[5], &BTreeSet::new());
    assert!(batch.starts_with("### ENTRY 5 (assistant)\n"));
    for ctx in ["2", "3", "4"] {
        assert!(batch.contains(&format!("[context #{ctx} ")));
    }
    assert!(!batch.contains("[context #1 "));
    assert!(batch.contains(&"x".repeat(TARGET_CHARS)));
    assert!(!batch.contains(&"x".repeat(TARGET_CHARS + 1)));
}

#[test]
fn first_entry_has_no_context() {
    let entries = texted(2, |i| format!("m{i}"));
    assert!(!build_batch(&entries, &[0], &BTreeSet::new()).contains("[context"));
}

#[test]
fn reply_parses_with_fences_and_prose_around_the_array() {
    let reply = "Here:\n```json\n[{\"n\":4,\"rule\":true,\"approval\":false},{\"n\":5,\"rule\":false,\"approval\":true}]\n```";
    let parsed = parse_reply(reply, &[4, 5]);
    assert_eq!(
        parsed.tagged,
        vec![(4, flags(true, false)), (5, flags(false, true))]
    );
    assert!(parsed.missing.is_empty());
}

#[test]
fn reply_keeps_valid_rows_and_reports_missing_wrong_and_malformed_ones() {
    let reply = "[{\"n\":4,\"rule\":true,\"approval\":false},{\"n\":5,\"rule\":true},{\"n\":99,\"rule\":true,\"approval\":true}]";
    let parsed = parse_reply(reply, &[4, 5, 6]);
    assert_eq!(parsed.tagged, vec![(4, flags(true, false))]);
    assert_eq!(parsed.missing, vec![5, 6]);
}

#[test]
fn reply_that_is_not_an_array_misses_every_entry() {
    let parsed = parse_reply("I cannot do that.", &[1, 2]);
    assert!(parsed.tagged.is_empty());
    assert_eq!(parsed.missing, vec![1, 2]);
}

#[test]
fn duplicate_reply_rows_count_once() {
    let one = "{\"n\":4,\"rule\":true,\"approval\":false}";
    let parsed = parse_reply(&format!("[{one},{one}]"), &[4]);
    assert_eq!(parsed.tagged.len(), 1);
}

#[test]
fn ladder_goes_twenty_five_one_then_stops() {
    assert_eq!(BATCH_LADDER, [20, 5, 1]);
    assert_eq!(next_size(20), Some(5));
    assert_eq!(next_size(5), Some(1));
    assert_eq!(next_size(1), None);
}

#[test]
fn batches_split_ids_by_size() {
    let ids: Vec<usize> = (0..7).collect();
    let sizes: Vec<usize> = batches(&ids, 5).iter().map(Vec::len).collect();
    assert_eq!(sizes, [5, 2]);
}

#[test]
fn an_entry_is_skipped_on_its_second_failed_pass() {
    assert_eq!(failure_kind(0), RowKind::Failed);
    assert_eq!(failure_kind(1), RowKind::Skipped);
    assert_eq!(failure_kind(5), RowKind::Skipped);
}

#[test]
fn failures_never_count_as_tags() {
    let entries = texted(4, |i| format!("m{i}"));
    let s = state(&format!(
        "{}{}",
        encode_start(0, "t").unwrap(),
        row(2, &RowKind::Failed)
    ));
    assert_eq!(untagged(&entries, &s), vec![0, 1, 2, 3]);
}

#[test]
fn prompt_asks_two_labels_only() {
    assert!(TAG_PROMPT.contains("{\"n\":<id>,\"rule\":bool,\"approval\":bool}"));
    assert!(!TAG_PROMPT.contains("risk:") && !TAG_PROMPT.contains("topic_break"));
}

#[test]
fn labels_parse_and_select_flagged_ids() {
    assert_eq!(Label::parse("rule"), Ok(Label::Rule));
    assert_eq!(Label::parse(" approval "), Ok(Label::Approval));
    assert!(Label::parse("risk")
        .unwrap_err()
        .contains("rule or approval"));
    let text = [
        row(3, &RowKind::Tagged(flags(true, false))),
        row(4, &RowKind::Tagged(flags(false, true))),
        row(5, &RowKind::Tagged(flags(false, false))),
        row(6, &RowKind::Failed),
    ]
    .concat();
    let state = state(&text);
    let ids = |label| ids_with(&state, label).into_iter().collect::<Vec<_>>();
    assert_eq!(ids(Label::Rule), vec![3]);
    assert_eq!(ids(Label::Approval), vec![4]);
}

#[test]
fn a_later_start_row_moves_the_start_point() {
    let text = [encode_start(2, "t").unwrap(), encode_start(9, "t").unwrap()].concat();
    assert_eq!(state(&text).start, Some(9));
}

#[test]
fn an_exit_zero_call_gives_its_trimmed_reply() {
    assert_eq!(reply_text(Some("  [{\"n\":1}]\n")), "[{\"n\":1}]");
    assert_eq!(reply_text(None), "");
    assert_eq!(reply_text(Some("   ")), "");
}

#[test]
fn an_outage_or_a_stop_is_an_error_and_any_other_exit_gives_a_reply() {
    let reply = |failure, terminated, code| tag_reply(failure, terminated, code, Some("[]"));
    assert!(reply(Some(FailureKind::Outage), false, 1).is_err());
    assert!(reply(None, true, 0).is_err());
    assert!(reply(None, true, 1).is_err());
    let ok = |text: &str| CallReply {
        text: text.to_string(),
        exited_ok: true,
    };
    let bad = CallReply {
        text: String::new(),
        exited_ok: false,
    };
    assert_eq!(reply(Some(FailureKind::Refusal), false, 1), Ok(bad.clone()));
    assert_eq!(reply(None, false, 1), Ok(bad));
    assert_eq!(reply(None, false, 0), Ok(ok("[]")));
    assert_eq!(tag_reply(None, false, 0, None), Ok(ok("")));
}

#[test]
fn a_pass_retakes_the_lock_only_when_the_log_grew_after_its_last_read() {
    assert!(retake(5, 4));
    assert!(!retake(4, 4));
    assert!(!retake(3, 4));
}

#[test]
fn on_a_failed_call_the_api_error_lines_of_the_reply_count_like_stderr() {
    let kind = |stderr: &str, response: &str| tag_failure_kind(false, 1, stderr, response);
    assert_eq!(kind("", "API Error: 529 overloaded"), FailureKind::Outage);
    assert_eq!(
        kind("", "x\n\n api error: 503 unavailable"),
        FailureKind::Outage
    );
    assert_eq!(
        kind("", "see the API Error: 529 note"),
        FailureKind::Refusal
    );
    assert_eq!(kind("", r#"[{"n":503,"rule":true}]"#), FailureKind::Refusal);
    assert_eq!(kind("API Error: 529 overloaded", ""), FailureKind::Outage);
    assert_eq!(kind("read ECONNRESET", "[]"), FailureKind::Outage);
    assert_eq!(
        tag_failure_kind(true, 1, "", r#"[{"n":503}]"#),
        FailureKind::Timeout
    );
}

#[test]
fn an_exit_zero_reply_is_never_an_error() {
    let quoted = "API Error: 529 overloaded";
    assert_eq!(tag_failure_kind(false, 0, "", quoted), FailureKind::Refusal);
    let reply = tag_reply(None, false, 0, Some("API Error: 400 bad request")).unwrap();
    assert!(reply.exited_ok);
    assert_eq!(reply.text, "API Error: 400 bad request");
}

fn feed(calls: &[bool]) -> (PassHealth, Vec<Verdict>) {
    calls
        .iter()
        .fold((PassHealth::default(), vec![]), |(health, mut seen), ok| {
            let (health, verdict) = after_call(health, *ok);
            seen.push(verdict);
            (health, seen)
        })
}

#[test]
fn four_bad_calls_in_a_row_stop_the_pass_even_after_good_calls() {
    let (_, verdicts) = feed(&[false; 4]);
    assert_eq!(verdicts.last(), Some(&Verdict::Stop));
    assert!(verdicts[..3].iter().all(|v| *v == Verdict::Go));
    let (_, verdicts) = feed(&[true, true, false, false, false, false]);
    assert_eq!(verdicts.last(), Some(&Verdict::Stop));
    assert!(verdicts[..5].iter().all(|v| *v == Verdict::Go));
}

#[test]
fn a_good_call_resets_the_streak() {
    let (_, verdicts) = feed(&[false, false, false, true, false, false, false]);
    assert!(verdicts.iter().all(|v| *v == Verdict::Go));
}

#[test]
fn an_entry_isolated_by_a_bad_call_is_confirmed_only_by_a_later_good_call() {
    let health = isolated(PassHealth::default(), 7, false);
    let (health, confirmed) = take_confirmed(health);
    assert!(confirmed.is_empty());
    let (health, _) = after_call(health, true);
    let (_, confirmed) = take_confirmed(health);
    assert_eq!(confirmed, vec![7]);
}

#[test]
fn an_entry_left_out_of_an_exit_zero_reply_is_confirmed_at_once() {
    let (_, confirmed) = take_confirmed(isolated(PassHealth::default(), 3, true));
    assert_eq!(confirmed, vec![3]);
}

#[test]
fn a_stop_drops_the_entries_still_pending() {
    let mut health = isolated(PassHealth::default(), 7, false);
    for _ in 0..3 {
        health = after_call(health, false).0;
    }
    let (health, verdict) = after_call(health, false);
    assert_eq!(verdict, Verdict::Stop);
    assert!(take_confirmed(health).1.is_empty());
}

#[test]
fn stored_failures_and_skips_are_excluded_from_context() {
    let state = state(
        &[
            row(2, &RowKind::Failed),
            row(4, &RowKind::Skipped),
            row(5, &RowKind::Tagged(flags(true, false))),
        ]
        .concat(),
    );
    assert_eq!(context_exclusions(&state), BTreeSet::from([2, 4]));
}

#[test]
fn excluded_entries_never_ride_as_context() {
    let entries = texted(6, |i| format!("m{i}"));
    let batch = build_batch(&entries, &[5], &BTreeSet::from([3]));
    assert!(batch.contains("[context #4 ") && batch.contains("[context #2 "));
    assert!(!batch.contains("[context #3 "));
    let target = build_batch(&entries, &[3], &BTreeSet::from([3]));
    assert!(target.starts_with("### ENTRY 3 "));
}

#[test]
fn decision_is_not_a_label() {
    assert!(Label::parse("decision").is_err());
    assert!(!TAG_PROMPT.contains("decision"));
    let line = row(3, &RowKind::Tagged(flags(true, true)));
    assert!(!line.contains("decision"), "{line}");
}

#[test]
fn reply_with_only_rule_and_approval_parses_and_a_decision_key_is_ignored() {
    let reply = "[{\"n\":4,\"rule\":true,\"approval\":false},{\"n\":5,\"rule\":false,\"decision\":true,\"approval\":true}]";
    let parsed = parse_reply(reply, &[4, 5]);
    assert_eq!(parsed.tagged.len(), 2);
    assert!(parsed.tagged[0].1.rule && !parsed.tagged[0].1.approval);
    assert!(!parsed.tagged[1].1.rule && parsed.tagged[1].1.approval);
    assert!(parsed.missing.is_empty());
}

#[test]
fn an_old_row_with_a_decision_key_decodes_without_it() {
    let old = r#"{"v":1,"n":4,"rule":true,"decision":true,"approval":false,"model":"m","prompt":"v1","at":"t"}"#;
    let s = state(&format!("{old}\n"));
    assert_eq!(s.tags.get(&4).copied(), Some(flags(true, false)));
}

#[test]
fn a_setting_that_cannot_be_read_skips_the_pass_without_touching_the_store() {
    assert_eq!(tag_plan(Err::<bool, &str>("bad toml")), TagPlan::Skip);
    assert_eq!(tag_plan(Ok::<bool, &str>(true)), TagPlan::Run);
    assert_eq!(tag_plan(Ok::<bool, &str>(false)), TagPlan::SkipAhead);
}
