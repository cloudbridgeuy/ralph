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
