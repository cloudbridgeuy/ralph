pub(super) enum Shown {
    Line(String),
    Leaf { day: String, line: String },
    Unreadable(usize),
}

fn notice(first: usize, last: usize) -> String {
    if first == last {
        format!("[#{first}: unreadable log line, skipped]")
    } else {
        format!("[#{first}-{last}: unreadable log lines, skipped]")
    }
}

pub(super) fn merge_notices(shown: impl IntoIterator<Item = Shown>) -> Vec<String> {
    let mut lines = Vec::new();
    let mut run: Option<(usize, usize)> = None;
    let mut shown_day: Option<String> = None;
    for item in shown {
        let (line, day) = match item {
            Shown::Unreadable(id) => {
                run = match run {
                    Some((first, last)) if last + 1 == id => Some((first, id)),
                    Some((first, last)) => {
                        lines.push(notice(first, last));
                        Some((id, id))
                    }
                    None => Some((id, id)),
                };
                continue;
            }
            Shown::Line(line) => (line, None),
            Shown::Leaf { day, line } => (line, Some(day)),
        };
        if let Some((first, last)) = run.take() {
            lines.push(notice(first, last));
        }
        if let Some(day) = day.filter(|day| shown_day.as_ref() != Some(day)) {
            lines.push(format!("--- {day}"));
            shown_day = Some(day);
        }
        lines.push(line);
    }
    if let Some((first, last)) = run {
        lines.push(notice(first, last));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> Shown {
        Shown::Line(text.to_string())
    }

    #[test]
    fn a_single_unreadable_entry_is_one_notice() {
        let lines = merge_notices([line("a"), Shown::Unreadable(12), line("b")]);
        assert_eq!(lines, ["a", "[#12: unreadable log line, skipped]", "b"]);
    }

    #[test]
    fn consecutive_unreadable_entries_collapse_into_one_range() {
        let shown = [12, 13, 14].map(Shown::Unreadable);
        assert_eq!(
            merge_notices(shown),
            ["[#12-14: unreadable log lines, skipped]"]
        );
    }

    #[test]
    fn unreadable_entries_that_do_not_touch_stay_separate() {
        let shown = [Shown::Unreadable(2), line("x"), Shown::Unreadable(4)];
        assert_eq!(
            merge_notices(shown),
            [
                "[#2: unreadable log line, skipped]",
                "x",
                "[#4: unreadable log line, skipped]"
            ]
        );
        let gap = [Shown::Unreadable(2), Shown::Unreadable(5)];
        assert_eq!(merge_notices(gap).len(), 2);
    }

    #[test]
    fn nothing_unreadable_changes_nothing() {
        assert_eq!(merge_notices([line("a"), line("b")]), ["a", "b"]);
        assert!(merge_notices(Vec::new()).is_empty());
    }
}
