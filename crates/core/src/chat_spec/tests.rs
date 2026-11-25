use super::*;
use crate::chat_help::Command;

fn texts(found: Option<SpecCompletion>) -> Vec<String> {
    found.map_or_else(Vec::new, |found| {
        found.values.into_iter().map(|value| value.text).collect()
    })
}

#[test]
fn a_slash_offers_every_command_name_with_a_space() {
    let found = spec_completions("/", 1);
    assert_eq!(texts(found.clone()).len(), Command::ALL.len());
    assert!(found.is_some_and(|found| found.values.iter().all(|value| value.append_space)));
}

#[test]
fn a_partial_command_name_is_replaced_from_its_slash() {
    let found = spec_completions("  /th", 5);
    assert_eq!(found.as_ref().map(|found| found.span.clone()), Some(2..5));
    assert_eq!(texts(found), ["/thread"]);
}

#[test]
fn no_space_is_added_before_existing_whitespace() {
    let found = spec_completions("/th x", 3);
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
        assert_eq!(spec_completions(line, line.len()), None, "{line}");
    }
}
