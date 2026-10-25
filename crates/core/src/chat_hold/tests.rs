use super::*;
use crate::worker::Report;

fn entry(source: &str) -> Entry {
    Entry {
        source: source.to_string(),
        report: Report {
            worker: source.to_string(),
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

#[test]
fn parse_rejects_extra_tokens() {
    assert_eq!(HoldFilter::parse("a b"), None);
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
