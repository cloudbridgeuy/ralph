use super::*;
use crate::chat_memory::fixtures::texted;
use crate::chat_memory::Entry;
use crate::retry::FailureKind;

const MODEL: &str = "claude-haiku-5-5";

fn flags(rule: bool, decision: bool, approval: bool) -> Flags {
    Flags {
        rule,
        decision,
        approval,
    }
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
    let line = row(3, &RowKind::Tagged(flags(true, false, true)));
    assert!(line.ends_with('\n'));
    let value: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(value["v"], 1);
    assert_eq!(value["n"], 3);
    assert_eq!(value["rule"], true);
    assert_eq!(value["decision"], false);
    assert_eq!(value["model"], MODEL);
    assert_eq!(value["prompt"], PROMPT_VERSION);
    assert_eq!(
        state(&line).tags.get(&3).copied(),
        Some(flags(true, false, true))
    );
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
        row(1, &RowKind::Tagged(flags(false, false, false))),
        row(1, &RowKind::Tagged(flags(false, true, false))),
    );
    let s = state(&text);
    assert_eq!(s.tags.get(&1).copied(), Some(flags(false, true, false)));
    assert_eq!(s.tags.len(), 1);
}

#[test]
fn untagged_covers_start_to_end_and_skips_done_and_corrupt() {
    let mut entries = texted(8, |i| format!("m{i}"));
    entries[6] = Entry::corrupt();
    let s = state(&format!(
        "{}{}{}",
        encode_start(2, "t").unwrap(),
        row(3, &RowKind::Tagged(flags(false, false, false))),
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
    let batch = build_batch(&entries, &[5]);
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
    assert!(!build_batch(&entries, &[0]).contains("[context"));
}

#[test]
fn reply_parses_with_fences_and_prose_around_the_array() {
    let reply = "Here:\n```json\n[{\"n\":4,\"rule\":true,\"decision\":false,\"approval\":false},{\"n\":5,\"rule\":false,\"decision\":false,\"approval\":true}]\n```";
    let parsed = parse_reply(reply, &[4, 5]);
    assert_eq!(
        parsed.tagged,
        vec![
            (4, flags(true, false, false)),
            (5, flags(false, false, true))
        ]
    );
    assert!(parsed.missing.is_empty());
}

#[test]
fn reply_keeps_valid_rows_and_reports_missing_wrong_and_malformed_ones() {
    let reply = "[{\"n\":4,\"rule\":true,\"decision\":false,\"approval\":false},{\"n\":5,\"rule\":true},{\"n\":99,\"rule\":true,\"decision\":true,\"approval\":true}]";
    let parsed = parse_reply(reply, &[4, 5, 6]);
    assert_eq!(parsed.tagged, vec![(4, flags(true, false, false))]);
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
    let one = "{\"n\":4,\"rule\":true,\"decision\":false,\"approval\":false}";
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
fn prompt_asks_three_labels_only() {
    assert!(TAG_PROMPT.contains("{\"n\":<id>,\"rule\":bool,\"decision\":bool,\"approval\":bool}"));
    assert!(!TAG_PROMPT.contains("risk:") && !TAG_PROMPT.contains("topic_break"));
}

#[test]
fn labels_parse_and_select_flagged_ids() {
    assert_eq!(Label::parse("rule"), Ok(Label::Rule));
    assert_eq!(Label::parse(" approval "), Ok(Label::Approval));
    assert!(Label::parse("risk").unwrap_err().contains("rule, decision"));
    let text = [
        row(3, &RowKind::Tagged(flags(true, false, false))),
        row(4, &RowKind::Tagged(flags(false, true, true))),
        row(5, &RowKind::Tagged(flags(false, false, false))),
        row(6, &RowKind::Failed),
    ]
    .concat();
    let state = state(&text);
    let ids = |label| ids_with(&state, label).into_iter().collect::<Vec<_>>();
    assert_eq!(ids(Label::Rule), vec![3]);
    assert_eq!(ids(Label::Decision), vec![4]);
    assert_eq!(ids(Label::Approval), vec![4]);
}

#[test]
fn a_later_start_row_moves_the_start_point() {
    let text = [encode_start(2, "t").unwrap(), encode_start(9, "t").unwrap()].concat();
    assert_eq!(state(&text).start, Some(9));
}

#[test]
fn a_finished_call_gives_its_reply_and_a_failed_one_gives_nothing() {
    assert_eq!(reply_text(0, Some("  [{\"n\":1}]\n")), "[{\"n\":1}]");
    assert_eq!(reply_text(0, None), "");
    assert_eq!(reply_text(0, Some("   ")), "");
    assert_eq!(reply_text(1, Some("[{\"n\":1}]")), "");
}

#[test]
fn an_outage_or_a_stop_is_an_error_and_other_failures_are_empty_replies() {
    let reply = |failure, terminated, code| tag_reply(failure, terminated, code, Some("[]"));
    assert!(reply(Some(FailureKind::Outage), false, 1).is_err());
    assert!(reply(None, true, 0).is_err());
    assert!(reply(None, true, 1).is_err());
    assert_eq!(
        reply(Some(FailureKind::Refusal), false, 1),
        Ok(String::new())
    );
    assert_eq!(reply(None, false, 1), Ok(String::new()));
    assert_eq!(reply(None, false, 0), Ok("[]".to_string()));
    assert_eq!(tag_reply(None, false, 0, None), Ok(String::new()));
}

#[test]
fn a_pass_retakes_the_lock_only_when_the_log_grew_after_its_last_read() {
    assert!(retake(5, 4));
    assert!(!retake(4, 4));
    assert!(!retake(3, 4));
}

#[test]
fn entry_numbers_in_a_partial_reply_are_not_an_outage() {
    let kind =
        |timed_out, stderr: &str, response: &str| tag_failure_kind(timed_out, 1, stderr, response);
    assert_eq!(
        kind(false, "", r#"[{"n":503,"rule":true}]"#),
        FailureKind::Refusal
    );
    assert_eq!(kind(false, "", r#"[{"n":1529}]"#), FailureKind::Refusal);
    assert_eq!(kind(false, "", ""), FailureKind::Refusal);
    assert_eq!(
        kind(false, "API Error: 529 overloaded", ""),
        FailureKind::Outage
    );
    assert_eq!(kind(false, "read ECONNRESET", "[]"), FailureKind::Outage);
    assert_eq!(
        kind(false, "", "API Error: 503 Service Unavailable"),
        FailureKind::Outage
    );
    assert_eq!(
        kind(false, "", " api error: 529 overloaded"),
        FailureKind::Outage
    );
    assert_eq!(kind(true, "", r#"[{"n":503}]"#), FailureKind::Timeout);
}
