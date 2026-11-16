use super::tests::summarized;
use super::*;
use crate::chat_memory::fixtures::entry;

fn on_days(days: &[&str]) -> Vec<Entry> {
    days.iter()
        .enumerate()
        .map(|(i, day)| {
            let mut e = entry(i, 5);
            e.at = format!("{day}T0{}:00:00.123456+00:00", i % 10);
            e
        })
        .collect()
}

#[test]
fn tail_shows_a_day_header_before_the_first_message_of_each_day() {
    let entries = on_days(&["2026-01-01", "2026-01-01", "2026-01-02"]);
    assert_eq!(
        tail_lines(&entries, &Tree::new(), 0, 100),
        [
            "--- 2026-01-01",
            "#0 user m0 xxxxx",
            "#1 assistant m1 xxxxx",
            "--- 2026-01-02",
            "#2 user m2 xxxxx",
        ]
    );
}

#[test]
fn view_leaf_lines_carry_the_day_in_a_header_and_no_clock_time() {
    let entries = on_days(&["2026-01-01", "2026-01-02", "2026-01-02"]);
    assert_eq!(
        view(&entries, &Tree::new(), 200),
        [
            "--- 2026-01-01",
            "#0 user m0 xxxxx",
            "--- 2026-01-02",
            "#1 assistant m1 xxxxx",
            "#2 user m2 xxxxx",
        ]
    );
}

#[test]
fn summary_lines_get_no_day_header() {
    let entries = on_days(&["2026-01-01"; 4]);
    let tree = summarized(&entries);
    assert_eq!(
        view(&entries, &tree, 3),
        [
            "#0-1 sum 0-1",
            "--- 2026-01-01",
            "#2 user m2 xxxxx",
            "#3 assistant m3 xxxxx",
        ]
    );
}

#[test]
fn a_day_header_marks_each_day_change_after_a_summary() {
    let entries = on_days(&["2026-01-01", "2026-01-01", "2026-01-01", "2026-01-02"]);
    let tree = summarized(&entries);
    assert_eq!(
        view(&entries, &tree, 3),
        [
            "#0-1 sum 0-1",
            "--- 2026-01-01",
            "#2 user m2 xxxxx",
            "--- 2026-01-02",
            "#3 assistant m3 xxxxx",
        ]
    );
}

#[test]
fn each_view_part_starts_its_own_day_header() {
    let entries: Vec<Entry> = on_days(&["2026-01-01"; 300]);
    let tree = summarized(&entries);
    let split = split_view(&entries, &tree, 200, 30);
    assert!(!split.older.is_empty() && !split.tail.is_empty());
    assert!(split.middle.iter().all(|line| !line.starts_with("---")));
    assert_eq!(split.tail[0], "--- 2026-01-01");
    assert_eq!(
        split.tail.iter().filter(|l| l.starts_with("---")).count(),
        1
    );
}

#[test]
fn zoom_and_nap_lines_keep_the_full_timestamp() {
    let entries = on_days(&["2026-01-01"]);
    assert_eq!(
        leaf_line(0, &entries[0], &Tree::new()).unwrap(),
        "#0 user 2026-01-01T00:00:00.123456+00:00 m0 xxxxx"
    );
    assert_eq!(
        entry_line(0, &entries[0], "body"),
        "#0 user 2026-01-01T00:00:00.123456+00:00 body"
    );
}
