use super::*;
use crate::chat_help::Command;
use crate::chat_thread::ThreadName;

fn texts(found: Option<SpecCompletion>) -> Vec<String> {
    found.map_or_else(Vec::new, |found| {
        found.values.into_iter().map(|value| value.text).collect()
    })
}

#[test]
fn a_slash_offers_every_command_name_with_a_space() {
    let found = spec_completions("/", 1, &CompletionFacts::default());
    assert_eq!(texts(found.clone()).len(), Command::ALL.len());
    assert!(found.is_some_and(|found| found.values.iter().all(|value| value.append_space)));
}

#[test]
fn a_partial_command_name_is_replaced_from_its_slash() {
    let found = spec_completions("  /th", 5, &CompletionFacts::default());
    assert_eq!(found.as_ref().map(|found| found.span.clone()), Some(2..5));
    assert_eq!(texts(found), ["/thread"]);
}

#[test]
fn no_space_is_added_before_existing_whitespace() {
    let found = spec_completions("/th x", 3, &CompletionFacts::default());
    assert_eq!(
        found.map(|found| found.values),
        Some(vec![SpecValue {
            text: "/thread".to_string(),
            append_space: false,
        }])
    );
}

#[test]
fn paths_and_prose_are_left_to_other_completers() {
    for line in ["/Us", "/Users/", "/thread/x", "see /th", "th", ""] {
        let found = spec_completions(line, line.len(), &CompletionFacts::default());
        assert_eq!(found, None, "{line}");
    }
}

fn complete(line: &str, current: Option<&str>) -> Option<Vec<String>> {
    let current = current.map(|name| ThreadName::parse(name).unwrap());
    let threads = ["auth", "side", "sub"].map(String::from);
    let facts = CompletionFacts {
        current: current.as_ref(),
        threads: &threads,
    };
    let found = spec_completions(line, line.len(), &facts);
    found.map(|found| texts(Some(found)))
}

fn words(list: &[&str]) -> Option<Vec<String>> {
    Some(list.iter().map(ToString::to_string).collect())
}

#[test]
fn switch_completion_lists_open_threads_without_the_current_one() {
    assert_eq!(
        complete("/thread switch ", Some("side")),
        words(&["auth", "sub"])
    );
    assert_eq!(complete("/thread switch s", None), words(&["side", "sub"]));
}

#[test]
fn subcommand_completion_offers_main_only_inside_a_thread() {
    let inside = [
        "drop", "ls", "main", "merge", "open", "park", "pull", "revive", "switch",
    ];
    assert_eq!(complete("/thread ", Some("side")), words(&inside));
    assert_eq!(
        complete("/thread m", Some("side")),
        words(&["main", "merge"])
    );
    assert_eq!(complete("/thread m", None), words(&["merge"]));
}

#[test]
fn completion_stops_after_the_name_and_where_nothing_is_offered() {
    for line in [
        "/thread switch auth ",
        "/thread park ",
        "/thread open ",
        "/thread revive ",
        "/main ",
    ] {
        assert_eq!(complete(line, Some("side")), None, "{line}");
    }
}

#[test]
fn subcommands_take_a_space_and_thread_names_do_not() {
    let spaced = |line: &str| {
        let threads = ["auth".to_string()];
        let facts = CompletionFacts {
            current: None,
            threads: &threads,
        };
        spec_completions(line, line.len(), &facts)
            .map(|found| (found.span, found.values[0].append_space))
    };
    assert_eq!(spaced("/thread sw"), Some((8..10, true)));
    assert_eq!(spaced("/thread switch au"), Some((15..17, false)));
}
