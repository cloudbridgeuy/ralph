use super::*;

#[test]
fn bare_rerender_means_the_last_reply() {
    assert_eq!(parse_rerender_args(""), Ok(1));
    assert_eq!(parse_rerender_args("   "), Ok(1));
}

#[test]
fn a_positive_number_selects_that_reply() {
    assert_eq!(parse_rerender_args("1"), Ok(1));
    assert_eq!(parse_rerender_args(" 3 "), Ok(3));
}

#[test]
fn zero_words_and_extra_arguments_give_the_usage() {
    let usage = Err(RERENDER_USAGE.to_string());
    for args in ["0", "x", "-1", "1.5", "2 3", "1 x", "last"] {
        assert_eq!(parse_rerender_args(args), usage, "{args}");
    }
    assert_eq!(
        RERENDER_USAGE,
        "Usage: /rerender [N] (N is 1 or more; 1 is the last reply)"
    );
}

fn entry(role: Role, text: &str) -> Entry {
    Entry {
        role,
        at: String::new(),
        text: text.to_string(),
        corrupt: false,
    }
}

fn log() -> Vec<Entry> {
    vec![
        entry(Role::User, "q1"),
        entry(Role::Assistant, "a1"),
        entry(Role::User, "q2"),
        entry(Role::Assistant, "a2"),
        entry(Role::Worker, "w"),
        Entry::corrupt(),
        entry(Role::User, "q3"),
        entry(Role::Assistant, "a3"),
        entry(Role::Worker, "w2"),
    ]
}

#[test]
fn n_counts_assistant_replies_from_the_end() {
    let entries = log();
    for (n, text) in [(1, "a3"), (2, "a2"), (3, "a1")] {
        assert_eq!(nth_last_assistant(&entries, n).unwrap().text, text);
    }
}

#[test]
fn worker_user_and_corrupt_entries_are_skipped() {
    let entries = vec![
        entry(Role::Assistant, "a1"),
        entry(Role::Worker, "w"),
        Entry::corrupt(),
        entry(Role::User, "q"),
    ];
    assert_eq!(nth_last_assistant(&entries, 1).unwrap().text, "a1");
    assert!(nth_last_assistant(&entries, 2).is_err());
}

#[test]
fn a_reply_past_the_start_names_the_count() {
    assert_eq!(
        nth_last_assistant(&log(), 4).unwrap_err(),
        "/rerender: no reply 4; the chat has 3 assistant replies."
    );
    assert_eq!(
        nth_last_assistant(&log(), 99).unwrap_err(),
        "/rerender: no reply 99; the chat has 3 assistant replies."
    );
}

#[test]
fn no_assistant_reply_says_so() {
    let none = "/rerender: no assistant reply yet.";
    assert_eq!(nth_last_assistant(&[], 1).unwrap_err(), none);
    let only_user = [entry(Role::User, "q"), Entry::corrupt()];
    assert_eq!(nth_last_assistant(&only_user, 2).unwrap_err(), none);
}

#[test]
fn the_selector_borrows_the_stored_entry() {
    let entries = log();
    let picked: &Entry = nth_last_assistant(&entries, 1).unwrap();
    assert!(std::ptr::eq(picked, &entries[7]));
}

#[test]
fn directive_tags_go_and_their_content_stays() {
    let wrapped = "<ralph-ask to=\"reviewer\">Please review.</ralph-ask>";
    assert_eq!(strip_directive_tags(wrapped), "Please review.");
    let two = "<ralph-tell to=\"a\">Hi\nthere</ralph-tell>\n\nplain\n\n<ralph-comment to=\"b\">Go</ralph-comment>";
    assert_eq!(strip_directive_tags(two), "Hi\nthere\n\nplain\n\nGo");
}

#[test]
fn text_without_directive_tags_is_unchanged() {
    for text in [
        "",
        "plain **bold** text",
        "a < b and c > d",
        "<div>html</div>",
        "<ralph-ask>no target</ralph-ask>",
        "<ralph- to=\"x\">no verb",
        "<ralph-ask to=\"x",
        "naïve — ünïcode <é>",
        "```\n<ralph\n```",
    ] {
        let expected = text.replace("</ralph-ask>", "");
        assert_eq!(strip_directive_tags(text), expected, "{text}");
    }
}

#[test]
fn an_unclosed_open_tag_loses_only_the_tag() {
    assert_eq!(
        strip_directive_tags("see <ralph-ask to=\"a\">half"),
        "see half"
    );
}

#[test]
fn the_header_names_the_reply_number() {
    assert_eq!(rerender_header(1), "reprint of reply 1");
    assert_eq!(rerender_header(12), "reprint of reply 12");
}

#[test]
fn a_reprint_pairs_the_header_with_the_stripped_markdown() {
    let entries = [
        entry(Role::Assistant, "old"),
        entry(Role::Assistant, "<ralph-ask to=\"a\">Q?</ralph-ask>"),
    ];
    let reprint = reprint(&entries, 1).unwrap();
    assert_eq!(reprint.header, "reprint of reply 1");
    assert_eq!(reprint.markdown, "Q?");
    assert_eq!(super::reprint(&entries, 2).unwrap().markdown, "old");
    assert_eq!(
        super::reprint(&entries, 3).unwrap_err(),
        "/rerender: no reply 3; the chat has 2 assistant replies."
    );
}
