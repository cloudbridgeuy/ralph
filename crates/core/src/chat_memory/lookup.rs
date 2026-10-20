use super::tree::{block_line, collapse, entry_line, leaf_line};
use super::{Entry, Node, Tree, Window};
use regex::RegexBuilder;

/// Entries that `zoom` reads: the leaf itself, or both leaves of a pair.
/// Larger blocks show only their halves' summaries.
pub fn zoom_entries(node: Node, total: usize) -> (usize, usize) {
    let start = node.lo.min(total);
    let end = if node.size() <= 2 {
        node.hi.min(total)
    } else {
        start
    };
    (start, end)
}

pub fn zoom(window: &Window, tree: &Tree, node: Node) -> Result<Vec<String>, String> {
    let t = window.total;
    if node.lo >= t {
        return Err(format!(
            "#{} is beyond the chat: it holds {t} entries.",
            node.id()
        ));
    }
    let entry = |i: usize| window.get(i).ok_or_else(|| format!("#{i} is not loaded."));
    if node.size() == 1 {
        let entry = entry(node.lo)?;
        return Ok(vec![entry_line(node.lo, entry, &entry.text)]);
    }
    let (left, right) = node.halves();
    [left, right]
        .into_iter()
        .filter(|half| half.lo < t)
        .map(|half| {
            let line = if half.size() == 1 {
                leaf_line(half.lo, entry(half.lo)?, tree)
            } else {
                block_line(tree, half)
            };
            Ok(line.unwrap_or_else(|| format!("#{} not compressed yet", half.id())))
        })
        .collect()
}

const RECALL_MAX_LINES: usize = 50;
const PREVIEW_CHARS: usize = 80;
const PREVIEW_LEAD_CHARS: usize = 20;

fn preview(text: &str, match_start: usize) -> String {
    let lead = collapse(&text[..match_start]).chars().count();
    let flat = collapse(text);
    let total = flat.chars().count();
    let from = lead.saturating_sub(PREVIEW_LEAD_CHARS);
    let to = (from + PREVIEW_CHARS).min(total);
    let from = to.saturating_sub(PREVIEW_CHARS).min(from);
    let body: String = flat.chars().skip(from).take(to - from).collect();
    let head = if from > 0 { "…" } else { "" };
    let tail = if to < total { "…" } else { "" };
    format!("{head}{body}{tail}")
}

fn date_of(at: &str) -> &str {
    at.split('T').next().unwrap_or(at)
}

fn format_recall(pattern: &str, hits: &[(usize, &Entry, String)]) -> String {
    if hits.is_empty() {
        return format!("no hits for {pattern}");
    }
    let mut lines: Vec<String> = hits
        .iter()
        .take(RECALL_MAX_LINES)
        .map(|(i, entry, preview)| {
            format!(
                "#{i} · {} · {} · {preview}",
                entry.role.as_str(),
                date_of(&entry.at)
            )
        })
        .collect();
    if hits.len() > RECALL_MAX_LINES {
        lines.push(format!(
            "{} more, narrow the pattern",
            hits.len() - RECALL_MAX_LINES
        ));
    }
    lines.push("(zoom <id> for the full text)".to_string());
    lines.join("\n")
}

pub fn recall(entries: &[Entry], pattern: &str) -> Result<String, String> {
    let re = RegexBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .map_err(|e| format!("bad regex: {e}"))?;
    let hits: Vec<(usize, &Entry, String)> = entries
        .iter()
        .enumerate()
        .rev()
        .filter_map(|(i, entry)| {
            re.find(&entry.text)
                .map(|m| (i, entry, preview(&entry.text, m.start())))
        })
        .collect();
    Ok(format_recall(pattern, &hits))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_memory::fixtures::{entry, n, texted};

    #[test]
    fn zoom_entries_loads_only_leaves_and_pairs() {
        assert_eq!(zoom_entries(n(5, 6), 9), (5, 6));
        assert_eq!(zoom_entries(n(8, 10), 9), (8, 9));
        assert_eq!(zoom_entries(n(0, 4), 9), (0, 0));
        assert_eq!(zoom_entries(n(16, 17), 9), (9, 9));
    }

    #[test]
    fn zoom_leaf_halves_and_beyond() {
        let mut entries = vec![entry(0, 5), entry(1, 5), entry(2, 5)];
        entries[1].text = "a\n  b".into();
        let mut tree = Tree::new();
        assert_eq!(
            zoom(&Window::full(&entries), &tree, n(1, 2)).unwrap(),
            ["#1 assistant 2026-01-01T00:00:01Z a\n  b"]
        );
        let halves = zoom(&Window::full(&entries), &tree, n(0, 4)).unwrap();
        assert_eq!(
            halves,
            ["#0-1 not compressed yet", "#2-3 not compressed yet"]
        );
        tree.insert(n(0, 2), "sum".into());
        assert_eq!(
            zoom(&Window::full(&entries), &tree, n(0, 4)).unwrap()[0],
            "#0-1 sum"
        );
        assert!(zoom(&Window::full(&entries), &tree, n(4, 8)).is_err());
        assert!(zoom(&Window::full(&entries), &tree, n(3, 4)).is_err());
    }

    #[test]
    fn recall_lists_newest_first_with_footer() {
        let entries = texted(3, |i| format!("note {i} needle"));
        let out = recall(&entries, "NEEDLE").unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], "#2 · user · 2026-01-01 · note 2 needle");
        assert!(lines[1].starts_with("#1 · ") && lines[2].starts_with("#0 · "));
        assert_eq!(lines[3], "(zoom <id> for the full text)");
    }

    #[test]
    fn recall_previews_window_around_match() {
        let text = format!(
            "START {} deep needle here {}",
            "a ".repeat(100),
            "b ".repeat(100)
        );
        let entries = texted(1, |_| text.clone());
        let out = recall(&entries, "needle").unwrap();
        let line = out.lines().next().unwrap();
        let body = line.splitn(4, " · ").nth(3).unwrap();
        assert!(body.contains("needle") && !body.contains("START"));
        assert!(body.starts_with('…') && body.ends_with('…'));
        assert!(body.chars().count() <= PREVIEW_CHARS + 2);
    }

    #[test]
    fn recall_preview_is_char_safe_and_untruncated_when_short() {
        let entries = texted(1, |_| "ñandú  needle\ncafé".into());
        let out = recall(&entries, "needle").unwrap();
        assert!(out.starts_with("#0 · user · 2026-01-01 · ñandú needle café\n"));
    }

    #[test]
    fn recall_caps_lines_and_counts_the_rest() {
        let entries = texted(53, |i| format!("hit {i}"));
        let out = recall(&entries, "hit").unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 52);
        assert!(lines[0].starts_with("#52 · "));
        assert!(lines[49].starts_with("#3 · "));
        assert_eq!(lines[50], "3 more, narrow the pattern");
        assert_eq!(lines[51], "(zoom <id> for the full text)");
    }

    #[test]
    fn recall_handles_no_hits_and_bad_regex() {
        let entries = texted(2, |i| format!("m{i}"));
        assert_eq!(recall(&entries, "zzz").unwrap(), "no hits for zzz");
        assert!(recall(&entries, "(").is_err());
    }

    #[test]
    fn date_of_cuts_at_t() {
        assert_eq!(date_of("2026-01-01T00:00:01Z"), "2026-01-01");
        assert_eq!(date_of("yesterday"), "yesterday");
    }
}
