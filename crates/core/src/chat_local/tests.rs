use super::*;
use crate::chat_status::{route_input, EmptySubmit};
use std::collections::BTreeSet;

fn route(line: &str) -> Route {
    route_input(line, 0, &EmptySubmit::default(), &BTreeSet::new())
}

fn rows(rows: &[&str]) -> Vec<String> {
    rows.iter().map(|row| row.to_string()).collect()
}

#[test]
fn commands_that_change_state_are_recorded() {
    for line in [
        "/send-it off",
        "/send-it go",
        "/send-it edit",
        "/send-it save nightly GUZ-1",
        "/send-it GUZ-1 nightly",
        "/thread merge auth",
        "/thread drop auth",
        "/thread park auth",
        "/thread revive auth",
        "/thread pull auth",
        "/thread merge",
        "/fork auth",
        "/watch start ci -- cargo test",
        "/watch stop w1",
        "/schedule cancel s1",
        "/inbox drop 1",
        "/nudge GUZ-1 keep going",
    ] {
        assert!(recorded(&route(line)), "{line}");
    }
}

#[test]
fn read_only_commands_and_messages_are_not_recorded() {
    for line in [
        "/send-it status",
        "/send-it templates",
        "/send-it",
        "/thread ls",
        "/thread auth",
        "/fork",
        "/watch list",
        "/watch",
        "/schedule list",
        "/inbox list",
        "/inbox peek 1",
        "/inbox",
        "/nudge",
        "/workers",
        "/hold",
        "/help",
        "/config",
        "/diff",
        "@GUZ-1 hi",
        "hello",
    ] {
        assert!(!recorded(&route(line)), "{line}");
    }
}

#[test]
fn a_record_holds_the_typed_line_and_its_result() {
    assert_eq!(
        local_record(
            "  /send-it off ",
            "No send-it run. Start one with /send-it <instructions>.\n"
        ),
        "/send-it off\nResult: No send-it run. Start one with /send-it <instructions>."
    );
}

#[test]
fn a_notice_record_names_ralph_as_its_source() {
    assert_eq!(
        notice_record("send-it done: 2/2 steps in 3 turns, $0.40.\n"),
        "[ralph] send-it done: 2/2 steps in 3 turns, $0.40."
    );
}

#[test]
fn records_and_notices_are_known_and_person_text_is_not() {
    assert!(is_local_record(&local_record(
        "/thread drop auth",
        "Dropped."
    )));
    assert!(is_local_record(&notice_record(
        "send-it stopped at step 1/2: x."
    )));
    for text in [
        "/send-it off",
        "please run /send-it off\nResult: none",
        "@GUZ-1 hi\nResult: Sent to worker GUZ-1; busy.",
        "merge it",
    ] {
        assert!(!is_local_record(text), "{text}");
    }
}

#[test]
fn catch_up_prompt_puts_unseen_rows_before_the_message() {
    assert_eq!(
        catch_up_prompt(&rows(&["7 user: /send-it off", "Result: gone"]), "next?"),
        "Since your last turn:\n7 user: /send-it off\nResult: gone\n\nNew message:\nnext?"
    );
}

#[test]
fn catch_up_prompt_without_rows_is_the_message() {
    assert_eq!(catch_up_prompt(&[], "next?"), "next?");
}

fn logged(texts: &[&str]) -> Vec<crate::chat_memory::Entry> {
    texts
        .iter()
        .enumerate()
        .map(|(i, text)| crate::chat_memory::Entry {
            role: crate::chat_memory::Role::User,
            at: format!("2026-01-01T00:00:0{i}Z"),
            text: text.to_string(),
            corrupt: false,
        })
        .collect()
}

#[test]
fn rows_after_the_seen_length_are_shown_as_view_lines() {
    let entries = logged(&["hi", "hello", "/send-it off\nResult: gone"]);
    assert_eq!(
        unseen_lines(&entries, Some(2)),
        rows(&["#2 user 2026-01-01T00:00:02Z /send-it off\nResult: gone"])
    );
}

#[test]
fn an_unknown_seen_length_or_a_caught_up_log_shows_nothing() {
    let entries = logged(&["hi", "hello"]);
    assert!(unseen_lines(&entries, None).is_empty());
    assert!(unseen_lines(&entries, Some(2)).is_empty());
    assert!(unseen_lines(&entries, Some(5)).is_empty());
}

#[test]
fn every_unseen_row_is_shown_in_log_order() {
    let entries = logged(&[
        "hi",
        "/thread drop a\nResult: Dropped a.",
        "@GUZ-1 go\nResult: Sent.",
    ]);
    assert_eq!(
        unseen_lines(&entries, Some(1)),
        rows(&[
            "#1 user 2026-01-01T00:00:01Z /thread drop a\nResult: Dropped a.",
            "#2 user 2026-01-01T00:00:02Z @GUZ-1 go\nResult: Sent.",
        ])
    );
}

#[test]
fn a_state_command_gives_its_log_row() {
    assert_eq!(
        CommandRecord::for_route(&route("/send-it off"), "/send-it off")
            .row("No send-it run. Start one with /send-it <instructions>."),
        Some(
            "/send-it off\nResult: No send-it run. Start one with /send-it <instructions>."
                .to_string()
        )
    );
    assert_eq!(
        CommandRecord::for_route(&route("/thread park auth"), "/thread park auth")
            .row("Parked auth."),
        Some("/thread park auth\nResult: Parked auth.".to_string())
    );
}

#[test]
fn a_read_only_command_gives_no_log_row() {
    assert_eq!(
        CommandRecord::for_route(&route("/send-it status"), "/send-it status")
            .row("No send-it run."),
        None
    );
    assert_eq!(
        CommandRecord::for_route(&route("/watch list"), "/watch list").row("No watches."),
        None
    );
}
