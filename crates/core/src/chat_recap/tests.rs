use super::*;

const NOW: &str = "2026-11-02T12:00:00Z";

fn entry(role: Role, at: &str, text: &str) -> Entry {
    Entry {
        role,
        at: at.to_string(),
        text: text.to_string(),
        corrupt: false,
    }
}

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(NOW)
        .unwrap()
        .with_timezone(&Utc)
}

fn facts(entries: &[Entry]) -> RecapFacts<'_> {
    RecapFacts {
        entries,
        now: now(),
        workers: 0,
        unpushed: 0,
        width: 80,
    }
}

fn session() -> Vec<Entry> {
    vec![
        entry(Role::User, "2026-11-02T09:00:00Z", "old question"),
        entry(Role::Assistant, "2026-11-02T09:01:00Z", "old answer"),
        entry(
            Role::User,
            "2026-11-02T09:58:00Z",
            "\nShip the recap\nwith tests",
        ),
        entry(
            Role::Assistant,
            "2026-11-02T10:00:00Z",
            "Done.\n\nNext step: \"run the live check\"",
        ),
    ]
}

#[test]
fn an_empty_log_gives_no_recap() {
    assert!(recap(&facts(&[])).is_empty());
}

#[test]
fn a_log_with_no_assistant_reply_gives_no_recap() {
    let entries = [entry(Role::User, "2026-11-02T10:00:00Z", "hello")];
    assert!(recap(&facts(&entries)).is_empty());
}

#[test]
fn recap_shows_age_last_user_line_and_next_step() {
    let entries = session();
    assert_eq!(
        recap(&facts(&entries)),
        [
            "last session · 2h ago",
            "  you: Ship the recap",
            "  next: \"run the live check\"",
        ]
    );
}

#[test]
fn live_line_lists_workers_and_unpushed_commits() {
    let entries = session();
    let facts = RecapFacts {
        workers: 2,
        unpushed: 3,
        ..facts(&entries)
    };
    assert_eq!(
        recap(&facts).last().map(String::as_str),
        Some("  live: 2 workers · 3 unpushed")
    );
}

#[test]
fn live_line_leaves_out_zero_counts() {
    let entries = session();
    let one_worker = RecapFacts {
        workers: 1,
        unpushed: 0,
        ..facts(&entries)
    };
    assert_eq!(
        recap(&one_worker).last().map(String::as_str),
        Some("  live: 1 worker")
    );
    assert_eq!(recap(&facts(&entries)).len(), 3);
}

#[test]
fn each_line_is_cut_to_the_given_width() {
    let entries = [
        entry(Role::User, "2026-11-02T11:59:30Z", "a fairly long question"),
        entry(Role::Assistant, "2026-11-02T11:59:40Z", "next: x"),
    ];
    let narrow = RecapFacts {
        width: 12,
        ..facts(&entries)
    };
    let lines = recap(&narrow);
    assert_eq!(lines[1], "  you: a fa…");
    assert!(lines.iter().all(|line| line.chars().count() <= 12));
}

#[test]
fn a_reply_without_a_next_step_leaves_out_the_next_line() {
    let entries = [
        entry(Role::User, "2026-11-02T10:59:55Z", "hi"),
        entry(Role::Assistant, "2026-11-02T11:00:00Z", "Hello."),
    ];
    assert_eq!(
        recap(&facts(&entries)),
        ["last session · 1h ago", "  you: hi"]
    );
}

#[test]
fn corrupt_entries_are_skipped() {
    let mut entries = session();
    entries.push(Entry::corrupt());
    assert_eq!(recap(&facts(&entries))[0], "last session · 2h ago");
}

#[test]
fn next_step_reads_the_last_next_line_in_any_common_form() {
    assert_eq!(next_step("a\nNext step: run it"), Some("run it"));
    assert_eq!(next_step("**Next step:** run it"), Some("run it"));
    assert_eq!(
        next_step("next: fix it, ~1h. Go?"),
        Some("fix it, ~1h. Go?")
    );
    assert_eq!(
        next_step("Next step: first\nmore\n- Next: second"),
        Some("second")
    );
}

#[test]
fn next_step_is_none_without_a_next_line_or_with_empty_text() {
    assert_eq!(next_step("Nothing to do."), None);
    assert_eq!(next_step("Next step:"), None);
    assert_eq!(next_step("The next steps are unclear"), None);
}
