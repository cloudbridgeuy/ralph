use super::*;
use crate::chat_status::{ScheduleState, Status, WatchState, WorkerState};
use crate::worker::Report;

fn entry(worker: &str) -> Entry {
    Entry {
        source: format!("20261025122100-000003-{worker}.json"),
        report: Report {
            worker: worker.to_string(),
            at: "t".to_string(),
            status: "done".to_string(),
            text: String::new(),
        },
    }
}

fn source(name: &str) -> HoldFilter {
    HoldFilter::Source(name.to_string())
}

#[test]
fn parse_without_args_is_any() {
    assert_eq!(HoldFilter::parse(""), Some(HoldFilter::Any));
    assert_eq!(HoldFilter::parse("  "), Some(HoldFilter::Any));
}

#[test]
fn parse_one_token_is_a_source() {
    assert_eq!(HoldFilter::parse(" w1 "), Some(source("w1")));
    assert_eq!(HoldFilter::parse("watch:ci"), Some(source("watch:ci")));
}

fn set(names: &[&str]) -> HoldFilter {
    HoldFilter::Set(names.iter().map(|name| name.to_string()).collect())
}

#[test]
fn parse_several_tokens_is_a_set_in_typed_order() {
    assert_eq!(HoldFilter::parse("b a"), Some(set(&["b", "a"])));
    assert_eq!(
        HoldFilter::parse("  a \t b  c "),
        Some(set(&["a", "b", "c"]))
    );
}

#[test]
fn parse_drops_duplicates_and_collapses_to_a_source() {
    assert_eq!(HoldFilter::parse("a b a"), Some(set(&["a", "b"])));
    assert_eq!(HoldFilter::parse("a a"), Some(source("a")));
}

#[test]
fn render_is_the_canonical_command() {
    assert_eq!(HoldFilter::Any.render(), "/hold");
    assert_eq!(source("a").render(), "/hold a");
    assert_eq!(set(&["a", "b"]).render(), "/hold a b");
}

#[test]
fn label_joins_a_set() {
    assert_eq!(set(&["a", "b"]).label(), "a, b");
}

#[test]
fn a_set_matches_any_member() {
    assert!(set(&["a", "b"]).matches(&entry("b")));
    assert!(!set(&["a", "b"]).matches(&entry("c")));
}

#[test]
fn a_schedule_member_of_a_set_matches_its_worker() {
    assert!(set(&["a", "schedule:x"]).matches(&entry("sched-x")));
}

#[test]
fn first_match_hits_any_member_of_a_set() {
    let entries = [entry("c"), entry("b"), entry("a")];
    let found = first_match(&entries, &set(&["a", "b"])).unwrap();
    assert_eq!(found.to_string(), "2");
}

#[test]
fn label_names_the_source_or_any() {
    assert_eq!(HoldFilter::Any.label(), "anyone");
    assert_eq!(source("watch:ci").label(), "watch:ci");
}

#[test]
fn any_matches_every_entry() {
    assert!(HoldFilter::Any.matches(&entry("w1")));
}

#[test]
fn source_matches_exactly() {
    assert!(source("w1").matches(&entry("w1")));
    assert!(!source("w1").matches(&entry("w10")));
    assert!(!source("w1").matches(&entry("W1")));
    assert!(!source("w1").matches(&entry("watch:w1")));
}

#[test]
fn a_schedule_tag_matches_its_scheduled_worker_too() {
    assert!(source("schedule:a").matches(&entry("schedule:a")));
    assert!(source("schedule:a").matches(&entry("sched-a")));
    assert!(!source("schedule:a").matches(&entry("sched-ab")));
    assert!(!source("sched-a").matches(&entry("schedule:a")));
}

#[test]
fn source_matches_the_worker_not_the_file_name() {
    let held = entry("w2");
    assert!(source("w2").matches(&held));
    assert!(!source(&held.source).matches(&held));
}

#[test]
fn first_match_is_none_without_entries() {
    assert_eq!(first_match(&[], &HoldFilter::Any), None);
}

#[test]
fn first_match_any_is_the_first_position() {
    let entries = [entry("a"), entry("b")];
    assert_eq!(
        first_match(&entries, &HoldFilter::Any),
        Some(Position::FIRST)
    );
}

#[test]
fn first_match_skips_other_sources_and_is_one_based() {
    let entries = [entry("a"), entry("b"), entry("b")];
    let found = first_match(&entries, &source("b")).unwrap();
    assert_eq!(found.to_string(), "2");
}

#[test]
fn first_match_is_none_when_no_source_matches() {
    assert_eq!(first_match(&[entry("a")], &source("b")), None);
}

#[test]
fn ctrl_q_and_ctrl_d_leave() {
    assert_eq!(hold_key(&[0x11]), Some(Leave));
    assert_eq!(hold_key(&[0x04]), Some(Leave));
}

#[test]
fn leave_key_is_found_among_other_bytes() {
    assert_eq!(hold_key(b"ab\x11"), Some(Leave));
}

#[test]
fn other_keys_do_not_leave() {
    assert_eq!(hold_key(b""), None);
    assert_eq!(hold_key(b"q"), None);
    assert_eq!(hold_key(&[0x03, 0x1b, b'\r']), None);
}

#[test]
fn redraw_returns_to_the_line_start_and_drops_the_newline() {
    assert_eq!(redraw("bar\n", 0, 80), "\r\x1b[Jbar");
    assert_eq!(redraw("bar", 80, 80), "\r\x1b[Jbar");
}

#[test]
fn redraw_climbs_over_the_rows_a_narrower_terminal_wrapped_the_old_bar_onto() {
    assert_eq!(redraw("bar", 120, 50), "\r\x1b[2A\x1b[Jbar");
    assert_eq!(redraw("bar", 50, 120), "\r\x1b[Jbar");
}

fn status(workers: &[&str], watches: &[&str], schedule: &[&str]) -> Status {
    Status {
        workers: workers
            .iter()
            .map(|id| WorkerState {
                id: id.to_string(),
                state: "running".to_string(),
            })
            .collect(),
        watches: watches
            .iter()
            .map(|id| WatchState {
                id: id.to_string(),
                left: None,
            })
            .collect(),
        schedule: schedule
            .iter()
            .map(|id| ScheduleState {
                id: id.to_string(),
                time: "09:00".to_string(),
            })
            .collect(),
        ..Status::default()
    }
}

fn complete(line: &str, status: &Status) -> Option<HoldCompletion> {
    hold_completions(line, line.len(), status)
}

fn values(line: &str, status: &Status) -> Vec<String> {
    complete(line, status).map(|c| c.values).unwrap_or_default()
}

#[test]
fn completion_offers_every_kind_sorted() {
    let status = status(&["GUZ-2", "GUZ-1"], &["watch:ci"], &["nightly"]);
    assert_eq!(
        complete("/hold ", &status),
        Some(HoldCompletion {
            span: 6..6,
            values: vec![
                "GUZ-1".to_string(),
                "GUZ-2".to_string(),
                "schedule:nightly".to_string(),
                "watch:ci".to_string(),
            ],
        })
    );
}

#[test]
fn completion_filters_by_the_partial_word() {
    let status = status(&["GUZ-1", "w1"], &["watch:ci", "watch:docs"], &["nightly"]);
    let completion = complete("/hold watch:c", &status).unwrap();
    assert_eq!(completion.span, 6..13);
    assert_eq!(completion.values, ["watch:ci"]);
    assert_eq!(values("/hold w", &status), ["w1", "watch:ci", "watch:docs"]);
    assert_eq!(values("/hold sch", &status), ["schedule:nightly"]);
    assert_eq!(values("/hold GUZ", &status), ["GUZ-1"]);
}

#[test]
fn completion_offers_each_kind_alone() {
    assert_eq!(values("/hold ", &status(&["w1"], &[], &[])), ["w1"]);
    assert_eq!(
        values("/hold ", &status(&[], &["watch:ci"], &[])),
        ["watch:ci"]
    );
    assert_eq!(values("/hold ", &status(&[], &[], &["a"])), ["schedule:a"]);
}

#[test]
fn completion_lists_a_running_schedule_once() {
    let status = status(&[], &["schedule:nightly"], &["nightly"]);
    assert_eq!(values("/hold ", &status), ["schedule:nightly"]);
}

#[test]
fn completion_with_nothing_to_offer_is_empty() {
    assert_eq!(
        complete("/hold ", &Status::default()),
        Some(HoldCompletion {
            span: 6..6,
            values: vec![],
        })
    );
    assert_eq!(
        values("/hold zz", &status(&["w1"], &[], &[])),
        Vec::<String>::new()
    );
}

#[test]
fn completion_allows_spaces_around_the_command() {
    let status = status(&["w1"], &[], &[]);
    let completion = complete("  /hold   w", &status).unwrap();
    assert_eq!(completion.span, 10..11);
    assert_eq!(completion.values, ["w1"]);
}

#[test]
fn completion_only_applies_to_the_hold_argument() {
    let status = status(&["w1"], &["watch:ci"], &["a"]);
    for line in [
        "",
        "/hold",
        "/holdw",
        "/hold w1 w",
        "/workers ",
        "hello /hold ",
        "/inbox ",
    ] {
        assert_eq!(complete(line, &status), None, "{line:?}");
    }
}

#[test]
fn completion_reads_only_the_text_before_the_cursor() {
    let status = status(&["w1"], &[], &[]);
    let completion = hold_completions("/hold w trailing", 7, &status).unwrap();
    assert_eq!(completion.span, 6..7);
    assert_eq!(completion.values, ["w1"]);
    assert_eq!(hold_completions("/hold w1", 3, &status), None);
}

#[test]
fn completion_ignores_a_cursor_inside_a_character() {
    let status = status(&["w1"], &[], &[]);
    assert_eq!(hold_completions("/hold é", 7, &status), None);
    assert_eq!(hold_completions("/hold w", 99, &status), None);
}

#[test]
fn targets_are_every_listed_worker_watches_and_schedules() {
    let mut status = status(&["GUZ-1", "GUZ-2", "GUZ-3"], &["watch:ci"], &["nightly"]);
    status.workers[1].state = "done".to_string();
    status.workers[2].state = "idle".to_string();
    let targets: Vec<String> = hold_targets(&status).into_iter().collect();
    assert_eq!(
        targets,
        ["GUZ-1", "GUZ-2", "GUZ-3", "schedule:nightly", "watch:ci"]
    );
}

#[test]
fn completion_offers_idle_and_done_workers() {
    let mut status = status(&["w1", "w2"], &[], &[]);
    status.workers[0].state = "idle".to_string();
    status.workers[1].state = "done".to_string();
    assert_eq!(values("/hold ", &status), ["w1", "w2"]);
}
