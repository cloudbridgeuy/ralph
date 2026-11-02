use super::*;
use crate::ansi::{DIM, RESET};
use crate::chat_hold::redraw as hold_redraw;

fn full() -> HoldProgress {
    HoldProgress {
        id: "GUZ-1162".to_string(),
        elapsed_secs: 14 * 60 + 5,
        activity: Some(ActivityState::Running),
        rebase: Some((9, 15)),
        commits: Some(3),
        hint: Some("cargo test".to_string()),
    }
}

const FULL_LINE: &str = "GUZ-1162 · 14m · busy · rebase 9/15 · 3 commits · cargo test";

#[test]
fn the_full_line_joins_every_segment() {
    assert_eq!(line(&full(), 120, false), FULL_LINE);
}

#[test]
fn elapsed_uses_seconds_minutes_and_hours() {
    let at = |secs| {
        line(
            &HoldProgress {
                elapsed_secs: secs,
                activity: None,
                rebase: None,
                commits: None,
                hint: None,
                ..full()
            },
            80,
            false,
        )
    };
    assert_eq!(at(45), "GUZ-1162 · 45s");
    assert_eq!(at(14 * 60), "GUZ-1162 · 14m");
    assert_eq!(at(3600 + 5 * 60), "GUZ-1162 · 1h05m");
}

#[test]
fn activity_words_map_and_none_is_omitted() {
    let with = |activity| {
        line(
            &HoldProgress {
                activity,
                rebase: None,
                commits: None,
                hint: None,
                ..full()
            },
            80,
            false,
        )
    };
    assert_eq!(with(Some(ActivityState::Running)), "GUZ-1162 · 14m · busy");
    assert_eq!(
        with(Some(ActivityState::Background)),
        "GUZ-1162 · 14m · busy"
    );
    assert_eq!(with(Some(ActivityState::Idle)), "GUZ-1162 · 14m · idle");
    assert_eq!(with(Some(ActivityState::Failed)), "GUZ-1162 · 14m · failed");
    assert_eq!(with(None), "GUZ-1162 · 14m");
}

#[test]
fn one_commit_is_singular() {
    let one = HoldProgress {
        commits: Some(1),
        hint: None,
        rebase: None,
        ..full()
    };
    assert_eq!(line(&one, 80, false), "GUZ-1162 · 14m · busy · 1 commit");
}

#[test]
fn narrow_widths_drop_segments_right_to_left() {
    let no_hint = "GUZ-1162 · 14m · busy · rebase 9/15 · 3 commits";
    let no_commits = "GUZ-1162 · 14m · busy · rebase 9/15";
    let cases = [
        (60, FULL_LINE),
        (
            59,
            "GUZ-1162 · 14m · busy · rebase 9/15 · 3 commits · cargo te…",
        ),
        (52, "GUZ-1162 · 14m · busy · rebase 9/15 · 3 commits · c…"),
        (51, no_hint),
        (47, no_hint),
        (46, no_commits),
        (35, no_commits),
        (34, "GUZ-1162 · 14m · busy"),
        (21, "GUZ-1162 · 14m · busy"),
        (20, "GUZ-1162 · 14m"),
        (14, "GUZ-1162 · 14m"),
        (13, "GUZ-1162"),
        (8, "GUZ-1162"),
    ];
    for (width, expected) in cases {
        let shown = line(&full(), width, false);
        assert_eq!(shown, expected, "width {width}");
    }
}

#[test]
fn the_id_truncates_with_an_ellipsis_only_when_it_cannot_fit() {
    assert_eq!(line(&full(), 5, false), "GUZ-…");
    assert_eq!(line(&full(), 1, false), "…");
    assert_eq!(line(&full(), 0, false), "");
}

#[test]
fn width_counts_display_columns() {
    let wide = HoldProgress {
        id: "漢字漢字".to_string(),
        ..full()
    };
    assert_eq!(line(&wide, 8, false), "漢字漢字");
    assert_eq!(line(&wide, 7, false), "漢字漢…");
}

#[test]
fn color_off_is_plain_and_color_on_is_dim() {
    assert!(!line(&full(), 120, false).contains('\x1b'));
    assert_eq!(line(&full(), 120, true), format!("{DIM}{FULL_LINE}{RESET}"));
}

#[test]
fn rebase_progress_parses_trimmed_numbers() {
    assert_eq!(rebase_progress("9\n", " 15\n"), Some((9, 15)));
    assert_eq!(rebase_progress("x", "15"), None);
    assert_eq!(rebase_progress("9", ""), None);
    assert_eq!(rebase_progress("9", "0"), None);
}

#[test]
fn the_hint_is_the_last_non_blank_line_trimmed() {
    assert_eq!(
        last_hint("one\n  two words  \n\n   \n"),
        Some("two words".to_string())
    );
    assert_eq!(last_hint("\n  \n"), None);
    assert_eq!(last_hint(""), None);
    assert_eq!(last_hint("ok\x1b[31m red"), Some("ok[31m red".to_string()));
}

#[test]
fn lines_cap_at_three_then_count_the_rest() {
    let many: Vec<HoldProgress> = (0..5)
        .map(|n| HoldProgress {
            id: format!("W{n}"),
            activity: None,
            rebase: None,
            commits: None,
            hint: None,
            ..full()
        })
        .collect();
    assert_eq!(
        lines(&many, 80, false),
        ["W0 · 14m", "W1 · 14m", "W2 · 14m", "+2 more"]
    );
    assert_eq!(lines(&many[..3], 80, false).len(), 3);
    assert!(lines(&[], 80, false).is_empty());
    assert_eq!(
        lines(&many, 80, true).last().unwrap(),
        &format!("{DIM}+2 more{RESET}")
    );
}

#[test]
fn redraw_climbs_over_the_old_lines_then_the_bar_rows() {
    let shown = lines_of(&["a", "b"]);
    assert_eq!(redraw(&shown, 0, "BAR\n", 10, 80), "\r\x1b[Ja\nb\nBAR");
    assert_eq!(
        redraw(&shown, 2, "BAR\n", 100, 80),
        "\x1b[2A\r\x1b[1A\x1b[Ja\nb\nBAR"
    );
    assert_eq!(redraw(&[], 2, "BAR", 10, 80), "\x1b[2A\r\x1b[JBAR");
    assert_eq!(redraw(&[], 0, "BAR", 10, 80), hold_redraw("BAR", 10, 80));
}

fn lines_of(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_string()).collect()
}

#[test]
fn held_ids_follow_the_hold_filter() {
    let running: Vec<String> = ["A", "B", "sched-t1"].map(String::from).to_vec();
    let ids = |filter: HoldFilter| held_ids(&filter, &running);
    assert_eq!(ids(HoldFilter::Any), ["A", "B", "sched-t1"]);
    assert_eq!(ids(HoldFilter::Source("B".to_string())), ["B"]);
    assert_eq!(
        ids(HoldFilter::Set(vec!["B".to_string(), "A".to_string()])),
        ["A", "B"]
    );
    assert_eq!(
        ids(HoldFilter::Source("watch:ci".to_string())),
        Vec::<String>::new()
    );
    assert_eq!(
        ids(HoldFilter::Source("schedule:t1".to_string())),
        ["sched-t1"]
    );
}
