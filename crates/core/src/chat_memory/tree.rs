use super::nap::fallback_summary;
use super::{Entry, Frozen, Node, Tree, View, SUMMARY_BYTES};
use regex::RegexBuilder;

fn cover_at(t: usize, alpha: f64) -> Vec<(usize, usize)> {
    let root = t.next_power_of_two();
    let mut out = Vec::new();
    let mut stack = vec![(0, root)];
    while let Some((lo, hi)) = stack.pop() {
        if lo >= t {
            continue;
        }
        let size = hi - lo;
        if size > 1 && (hi > t || size as f64 > alpha * (t - hi + 1) as f64) {
            let mid = (lo + hi) / 2;
            stack.push((mid, hi));
            stack.push((lo, mid));
        } else {
            out.push((lo, hi));
        }
    }
    out.sort();
    out
}

pub fn cover(t: usize, budget: usize) -> Vec<Node> {
    if t == 0 {
        return Vec::new();
    }
    if t <= budget {
        return (0..t).map(Node::leaf).collect();
    }
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for _ in 0..60 {
        let mid = (lo + hi) / 2.0;
        if cover_at(t, mid).len() > budget {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let mut out = cover_at(t, hi);
    while out.len() < budget {
        let Some(i) = out.iter().rposition(|(lo, hi)| hi - lo > 1) else {
            break;
        };
        let (lo, hi) = out[i];
        let mid = (lo + hi) / 2;
        out.splice(i..=i, [(lo, mid), (mid, hi)]);
    }
    out.into_iter().map(|(lo, hi)| Node { lo, hi }).collect()
}

fn is_long(entry: &Entry) -> bool {
    entry.text.len() > SUMMARY_BYTES
}

pub fn pending(entries: &[Entry], tree: &Tree) -> Vec<Node> {
    let t = entries.len();
    let mut todo: Vec<Node> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| is_long(e))
        .map(|(i, _)| Node::leaf(i))
        .filter(|n| !tree.contains_key(n))
        .collect();
    let mut size = 2;
    while size <= t {
        todo.extend(
            (0..t / size)
                .map(|k| Node {
                    lo: k * size,
                    hi: (k + 1) * size,
                })
                .filter(|n| !tree.contains_key(n)),
        );
        size *= 2;
    }
    todo
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn entry_line(i: usize, entry: &Entry, body: &str) -> String {
    format!("#{i} {} {} {body}", entry.role.as_str(), entry.at)
}

pub(super) fn node_line(entries: &[Entry], tree: &Tree, node: Node) -> Option<String> {
    if node.size() == 1 {
        let entry = &entries[node.lo];
        if is_long(entry) {
            tree.get(&node).map(|s| entry_line(node.lo, entry, s))
        } else {
            Some(entry_line(node.lo, entry, &collapse(&entry.text)))
        }
    } else {
        tree.get(&node).map(|s| format!("#{} {s}", node.id()))
    }
}

/// Lines for a node whose summary may still be pending: a missing parent shows
/// its halves, a missing long leaf shows a marked raw excerpt that is never stored.
fn settled_lines(entries: &[Entry], tree: &Tree, node: Node) -> Vec<String> {
    if let Some(line) = node_line(entries, tree, node) {
        return vec![line];
    }
    if node.size() == 1 {
        let entry = &entries[node.lo];
        return vec![entry_line(node.lo, entry, &fallback_summary(&entry.text))];
    }
    let (left, right) = node.halves();
    [left, right]
        .into_iter()
        .flat_map(|half| settled_lines(entries, tree, half))
        .collect()
}

fn view(entries: &[Entry], tree: &Tree, budget: usize) -> View {
    let mut view = View::default();
    for node in cover(entries.len(), budget) {
        if node_line(entries, tree, node).is_none() {
            view.missing.push(node);
        }
        view.lines.extend(settled_lines(entries, tree, node));
    }
    view
}

pub fn frozen_block(
    entries: &[Entry],
    tree: &Tree,
    budget: usize,
    window: usize,
    pinned: Option<Frozen>,
) -> Frozen {
    let end = entries.len() / window * window;
    match pinned {
        Some(pinned) if pinned.end == end => pinned,
        _ => Frozen {
            end,
            lines: view(&entries[..end], tree, budget.saturating_sub(window)).lines,
        },
    }
}

pub fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(3)
}

pub fn verbatim_start(entries: &[Entry], from: usize, budget: usize) -> usize {
    let mut used = 0;
    let mut start = entries.len();
    for i in (from..entries.len()).rev() {
        used += estimate_tokens(&entries[i].text);
        if used > budget {
            break;
        }
        start = i;
    }
    start
}

pub fn tail_lines(entries: &[Entry], tree: &Tree, from: usize, budget: usize) -> Vec<String> {
    let start = verbatim_start(entries, from, budget);
    let settled = (from..start).flat_map(|i| settled_lines(entries, tree, Node::leaf(i)));
    let verbatim = (start..entries.len()).map(|i| entry_line(i, &entries[i], &entries[i].text));
    settled.chain(verbatim).collect()
}

pub fn zoom(entries: &[Entry], tree: &Tree, node: Node) -> Result<Vec<String>, String> {
    let t = entries.len();
    if node.lo >= t {
        return Err(format!(
            "#{} is beyond the chat: it holds {t} entries.",
            node.id()
        ));
    }
    if node.size() == 1 {
        let entry = &entries[node.lo];
        return Ok(vec![entry_line(node.lo, entry, &entry.text)]);
    }
    let (left, right) = node.halves();
    Ok([left, right]
        .into_iter()
        .filter(|half| half.lo < t)
        .map(|half| {
            node_line(entries, tree, half)
                .unwrap_or_else(|| format!("#{} not compressed yet", half.id()))
        })
        .collect())
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

pub(super) fn format_recall(pattern: &str, hits: &[(usize, &Entry, String)]) -> String {
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
    use crate::chat_memory::fixtures::{entry, n};
    use crate::chat_memory::FALLBACK_MARK;

    fn mixed(count: usize) -> Vec<Entry> {
        (0..count)
            .map(|i| entry(i, if i.is_multiple_of(3) { 600 } else { 5 }))
            .collect()
    }

    fn summarized(entries: &[Entry]) -> Tree {
        let mut tree = Tree::new();
        loop {
            let todo = pending(entries, &tree);
            if todo.is_empty() {
                return tree;
            }
            for node in todo {
                tree.insert(node, format!("sum {}", node.id()));
            }
        }
    }

    fn pairs(nodes: &[Node]) -> Vec<(usize, usize)> {
        nodes.iter().map(|n| (n.lo, n.hi)).collect()
    }

    #[test]
    fn cover_matches_python() {
        assert_eq!(
            pairs(&cover(10, 4)),
            [(0, 4), (4, 6), (6, 8), (8, 9), (9, 10)]
        );
        assert_eq!(
            pairs(&cover(100, 12)),
            [
                (0, 32),
                (32, 48),
                (48, 64),
                (64, 72),
                (72, 80),
                (80, 88),
                (88, 92),
                (92, 94),
                (94, 96),
                (96, 98),
                (98, 99),
                (99, 100)
            ]
        );
        assert_eq!(
            pairs(&cover(37, 8)),
            [
                (0, 16),
                (16, 24),
                (24, 28),
                (28, 32),
                (32, 34),
                (34, 35),
                (35, 36),
                (36, 37)
            ]
        );
        assert_eq!(
            pairs(&cover(64, 1)),
            [
                (0, 32),
                (32, 48),
                (48, 56),
                (56, 60),
                (60, 62),
                (62, 63),
                (63, 64)
            ]
        );
        assert_eq!(cover(5, 5).len(), 5);
        assert!(cover(0, 5).is_empty());
    }

    #[test]
    fn node_ids_round_trip_and_reject() {
        assert_eq!(Node::parse("5").unwrap(), n(5, 6));
        assert_eq!(Node::parse("4-7").unwrap(), n(4, 8));
        assert_eq!(n(5, 6).id(), "5");
        assert_eq!(n(4, 8).id(), "4-7");
        for bad in ["5-6", "0-2", "6-5", "a", "", "-3", "1-"] {
            assert!(Node::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn pending_lists_long_leaves_then_blocks() {
        let entries = vec![
            entry(0, 600),
            entry(1, 5),
            entry(2, 600),
            entry(3, 5),
            entry(4, 5),
        ];
        let mut tree = Tree::new();
        assert_eq!(
            pending(&entries, &tree),
            [n(0, 1), n(2, 3), n(0, 2), n(2, 4), n(0, 4)]
        );
        tree.insert(n(0, 1), "s".into());
        tree.insert(n(2, 4), "s".into());
        assert_eq!(pending(&entries, &tree), [n(2, 3), n(0, 2), n(0, 4)]);
    }

    #[test]
    fn zoom_leaf_halves_and_beyond() {
        let mut entries = vec![entry(0, 5), entry(1, 5), entry(2, 5)];
        entries[1].text = "a\n  b".into();
        let mut tree = Tree::new();
        assert_eq!(
            zoom(&entries, &tree, n(1, 2)).unwrap(),
            ["#1 assistant 2026-01-01T00:00:01Z a\n  b"]
        );
        let halves = zoom(&entries, &tree, n(0, 4)).unwrap();
        assert_eq!(
            halves,
            ["#0-1 not compressed yet", "#2-3 not compressed yet"]
        );
        tree.insert(n(0, 2), "sum".into());
        assert_eq!(zoom(&entries, &tree, n(0, 4)).unwrap()[0], "#0-1 sum");
        assert!(zoom(&entries, &tree, n(4, 8)).is_err());
        assert!(zoom(&entries, &tree, n(3, 4)).is_err());
    }

    fn texted(count: usize, text: impl Fn(usize) -> String) -> Vec<Entry> {
        (0..count)
            .map(|i| {
                let mut e = entry(i, 5);
                e.text = text(i);
                e
            })
            .collect()
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

    fn lines_cover(lines: &[String]) -> usize {
        lines
            .iter()
            .map(|line| {
                let id = line[1..].split(' ').next().unwrap();
                Node::parse(id).unwrap().size()
            })
            .sum()
    }

    #[test]
    fn pending_parent_shows_committed_halves_and_raw_excerpts() {
        let entries = vec![entry(0, 600), entry(1, 5), entry(2, 600), entry(3, 5)];
        let mut tree = Tree::new();
        tree.insert(n(0, 1), "leaf zero".into());
        tree.insert(n(0, 2), "pair zero".into());
        let lines = settled_lines(&entries, &tree, n(0, 4));
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "#0-1 pair zero");
        assert!(lines[1].starts_with("#2 user ") && lines[1].contains(FALLBACK_MARK));
        assert!(lines[1].len() <= SUMMARY_BYTES + 40);
        assert!(lines[2].starts_with("#3 assistant ") && lines[2].ends_with("m3 xxxxx"));
        assert_eq!(lines_cover(&lines), 4);
        assert_eq!(tree.len(), 2);
    }

    #[test]
    fn tail_shows_a_pending_long_leaf_as_raw_excerpt() {
        let entries = vec![entry(0, 600), entry(1, 5)];
        let lines = tail_lines(&entries, &Tree::new(), 0, 0);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains(FALLBACK_MARK));
        let mut tree = Tree::new();
        tree.insert(n(0, 1), "done".into());
        assert!(tail_lines(&entries, &tree, 0, 0)[0].ends_with(" done"));
    }

    #[test]
    fn estimate_rounds_bytes_up_by_three() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("a"), 1);
        assert_eq!(estimate_tokens("abc"), 1);
        assert_eq!(estimate_tokens("abcd"), 2);
        assert_eq!(estimate_tokens("ñ"), 1);
    }

    #[test]
    fn verbatim_start_takes_a_contiguous_newest_suffix() {
        let entries = texted(5, |_| "abcdef".into());
        assert_eq!(verbatim_start(&[], 0, 100), 0);
        assert_eq!(verbatim_start(&entries, 0, 0), 5);
        assert_eq!(verbatim_start(&entries, 0, 1), 5);
        assert_eq!(verbatim_start(&entries, 0, 4), 3);
        assert_eq!(verbatim_start(&entries, 0, 100), 0);
        assert_eq!(verbatim_start(&entries, 3, 100), 3);
    }

    #[test]
    fn verbatim_start_stops_at_the_first_entry_that_does_not_fit() {
        let mut entries = texted(4, |_| "ab".into());
        entries[1].text = "x".repeat(300);
        assert_eq!(verbatim_start(&entries, 0, 50), 2);
    }

    #[test]
    fn tail_keeps_multiline_messages_verbatim_within_budget() {
        let entries = texted(2, |i| format!("line one\n\n  line two {i}"));
        let lines = tail_lines(&entries, &Tree::new(), 0, 100);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("#0 user ") && lines[0].ends_with("line one\n\n  line two 0"));
        assert!(lines[1].ends_with("line one\n\n  line two 1"));
    }

    #[test]
    fn tail_rolls_older_messages_into_settled_form() {
        let entries = texted(4, |i| format!("a\nb {i}"));
        let lines = tail_lines(&entries, &Tree::new(), 0, 4);
        assert_eq!(lines[0], entry_line(0, &entries[0], "a b 0"));
        assert_eq!(lines[1], entry_line(1, &entries[1], "a b 1"));
        assert_eq!(lines[2], entry_line(2, &entries[2], "a\nb 2"));
        assert_eq!(lines[3], entry_line(3, &entries[3], "a\nb 3"));
    }

    #[test]
    fn tail_settles_an_oversized_newest_message() {
        let entries = vec![entry(0, 5), entry(1, 600)];
        let lines = tail_lines(&entries, &Tree::new(), 0, 10);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains(FALLBACK_MARK));
        let mut tree = Tree::new();
        tree.insert(n(1, 2), "short summary".into());
        assert!(tail_lines(&entries, &tree, 0, 10)[1].ends_with(" short summary"));
    }

    #[test]
    fn tail_with_zero_budget_matches_settled_lines() {
        let entries = texted(4, |i| format!("a\nb {i}"));
        let lines = tail_lines(&entries, &Tree::new(), 1, 0);
        assert_eq!(lines.len(), 3);
        assert!(lines.iter().all(|line| !line.contains('\n')));
        assert!(tail_lines(&[], &Tree::new(), 0, 100).is_empty());
    }

    #[test]
    fn tail_shows_each_entry_once_in_order() {
        let entries = mixed(9);
        for budget in [0, 5, 60, 400, 10_000] {
            let lines = tail_lines(&entries, &Tree::new(), 2, budget);
            let ids: Vec<usize> = lines
                .iter()
                .map(|line| line[1..line.find(' ').unwrap()].parse().unwrap())
                .collect();
            assert_eq!(ids, (2..9).collect::<Vec<_>>(), "budget {budget}");
        }
    }

    #[test]
    fn view_missing_then_complete() {
        let entries = mixed(40);
        let budget = 10;
        let mut tree = Tree::new();
        let before = view(&entries, &tree, budget);
        let cov = cover(entries.len(), budget);
        let expected: Vec<Node> = pending(&entries, &tree)
            .into_iter()
            .filter(|p| cov.contains(p))
            .collect();
        let mut got = before.missing.clone();
        got.sort();
        let mut want = expected;
        want.sort();
        assert_eq!(got, want);
        assert!(!got.is_empty());
        assert_eq!(lines_cover(&before.lines), entries.len());
        assert!(before.lines.iter().any(|line| line.contains(FALLBACK_MARK)));

        for node in pending(&entries, &tree) {
            tree.insert(node, format!("sum {}", node.id()));
        }
        assert!(pending(&entries, &tree).is_empty());
        let after = view(&entries, &tree, budget);
        assert!(after.missing.is_empty());
        assert!(after.lines.len() <= budget);
        let mut next = 0;
        for node in &cov {
            assert_eq!(node.lo, next);
            next = node.hi;
        }
        assert_eq!(next, entries.len());
        assert_eq!(after.lines.len(), cov.len());
    }

    fn shared_prefix(a: &[Node], b: &[Node]) -> usize {
        a.iter().zip(b).take_while(|(x, y)| x == y).count()
    }

    const VIEW_BUDGET: usize = 200;

    fn first_change(t: usize) -> usize {
        shared_prefix(&cover(t, VIEW_BUDGET), &cover(t + 1, VIEW_BUDGET))
    }

    #[test]
    fn cover_below_budget_only_appends() {
        for t in 1..VIEW_BUDGET {
            assert_eq!(first_change(t), t, "t={t}");
        }
    }

    #[test]
    fn cover_past_budget_rewrites_opening_nodes() {
        // The 201st entry merges leaves 0 and 1: the very first node changes.
        assert_eq!(cover(200, VIEW_BUDGET)[0], n(0, 1));
        assert_eq!(cover(201, VIEW_BUDGET)[0], n(0, 2));
        assert_eq!(first_change(200), 0);
        // Afterwards each append rewrites one node further in: node k = t - 200.
        for t in 201..268 {
            assert_eq!(first_change(t), t - 200, "t={t}");
        }
        // The first node changes again at 268 and 403, and nowhere else in 200..420.
        let moved: Vec<usize> = (200..420)
            .filter(|&t| cover(t, VIEW_BUDGET)[0] != cover(t + 1, VIEW_BUDGET)[0])
            .collect();
        assert_eq!(moved, [200, 268, 403]);
        // Later the stable prefix is longer but still not the whole view.
        assert_eq!(first_change(250), 50);
        assert_eq!(first_change(400), 100);
        assert_eq!(first_change(1000), 62);
    }

    #[test]
    fn view_opening_after_one_append() {
        // (entries before the append, first line unchanged, first 10 lines unchanged)
        let cases = [
            (150, true, true),
            (199, true, true),
            (200, false, false),
            (201, true, false),
            (250, true, true),
            (400, true, true),
        ];
        for (count, first_same, ten_same) in cases {
            let entries: Vec<Entry> = (0..=count).map(|i| entry(i, 5)).collect();
            let mut tree = Tree::new();
            for node in pending(&entries, &tree) {
                tree.insert(node, format!("sum {}", node.id()));
            }
            let before = view(&entries[..count], &tree, VIEW_BUDGET).lines;
            let after = view(&entries, &tree, VIEW_BUDGET).lines;
            let same = |k: usize| before.iter().take(k).eq(after.iter().take(k));
            assert_eq!(same(1), first_same, "first line at {count}");
            assert_eq!(same(10), ten_same, "first 10 lines at {count}");
        }
    }

    struct Split {
        frozen: Vec<String>,
        tail: Vec<String>,
    }

    fn split(entries: &[Entry], tree: &Tree, window: usize) -> Split {
        let frozen = frozen_block(entries, tree, 200, window, None);
        Split {
            tail: tail_lines(entries, tree, frozen.end, 0),
            frozen: frozen.lines,
        }
    }

    const WINDOWS: [usize; 2] = [32, 7];

    #[test]
    fn split_edges() {
        for w in WINDOWS {
            let tree = Tree::new();
            let none = split(&[], &tree, w);
            assert!(none.frozen.is_empty() && none.tail.is_empty());
            let few: Vec<Entry> = (0..w - 1).map(|i| entry(i, 5)).collect();
            let s = split(&few, &tree, w);
            assert!(s.frozen.is_empty());
            assert_eq!(s.tail.len(), w - 1);
            let exact: Vec<Entry> = (0..w * 2).map(|i| entry(i, 5)).collect();
            let s = split(&exact, &summarized(&exact), w);
            assert!(s.tail.is_empty());
            assert!(!s.frozen.is_empty());
        }
    }

    #[test]
    fn split_fits_budget_on_long_history() {
        let entries: Vec<Entry> = (0..1000).map(|i| entry(i, 5)).collect();
        let tree = summarized(&entries);
        for w in WINDOWS {
            let s = split(&entries, &tree, w);
            assert!(s.frozen.len() + s.tail.len() <= 200, "w={w}");
            let h = 1000 / w * w;
            assert_eq!(s.frozen, view(&entries[..h], &tree, 200 - w).lines);
            assert_eq!(s.tail.len(), 1000 - h);
            assert!(s.frozen[0].starts_with("#0-"));
        }
    }

    #[test]
    fn frozen_changes_only_at_window_boundaries() {
        let entries: Vec<Entry> = (0..1201).map(|i| entry(i, 5)).collect();
        let tree = summarized(&entries);
        for w in WINDOWS {
            let mut unchanged_past_200 = 0;
            for t in 1..=1200 {
                let a = split(&entries[..t], &tree, w);
                let b = split(&entries[..t + 1], &tree, w);
                if a.frozen == b.frozen {
                    assert_ne!((t + 1) % w, 0, "w={w} t={t}");
                    assert_eq!(b.tail[..a.tail.len()], a.tail[..], "w={w} t={t}");
                    if t >= 200 {
                        unchanged_past_200 += 1;
                    }
                } else {
                    assert_eq!((t + 1) % w, 0, "w={w} t={t}");
                }
            }
            let boundaries = (201..=1201).filter(|n| n % w == 0).count();
            assert_eq!(unchanged_past_200, 1001 - boundaries, "w={w}");
        }
    }

    #[test]
    fn frozen_block_rolls_over_at_every_window() {
        let entries: Vec<Entry> = (0..301).map(|i| entry(i, 5)).collect();
        let tree = summarized(&entries);
        for w in [1, 32, 7] {
            let mut pinned: Option<Frozen> = None;
            for t in 0..=300 {
                let block = frozen_block(&entries[..t], &tree, 200, w, pinned.take());
                assert_eq!(block.end, t / w * w, "w={w} t={t}");
                if let Some(previous) = &pinned {
                    assert_eq!(previous.end, block.end);
                }
                pinned = Some(block);
            }
            for t in 0..=300 - w {
                let end = t / w * w;
                let first = frozen_block(&entries[..end], &tree, 200, w, None);
                let mut pin = first.clone();
                for u in end..end + w {
                    pin = frozen_block(&entries[..u], &tree, 200, w, Some(pin));
                    assert_eq!(pin, first, "w={w} u={u}");
                }
                let rolled = frozen_block(&entries[..end + w], &tree, 200, w, Some(pin));
                assert_eq!(rolled.end, end + w, "w={w} t={t}");
            }
        }
    }

    #[test]
    fn pinned_block_survives_summaries_landing_mid_window() {
        let entries: Vec<Entry> = (0..400).map(|i| entry(i, 5)).collect();
        let landed = summarized(&entries);
        for w in WINDOWS {
            let h = 256 / w * w;
            let first = frozen_block(&entries[..h], &Tree::new(), 200, w, None);
            let mut pinned = first.clone();
            for t in h..h + w {
                pinned = frozen_block(&entries[..t], &landed, 200, w, Some(pinned));
                assert_eq!(pinned, first, "w={w} t={t}");
            }
            let unpinned = frozen_block(&entries[..h + 1], &landed, 200, w, None);
            assert_ne!(unpinned.lines, first.lines, "w={w}");
            let rolled = frozen_block(&entries[..h + w], &landed, 200, w, Some(pinned));
            assert_eq!(rolled.end, h + w);
            assert_ne!(rolled.lines, first.lines);
        }
    }

    #[test]
    fn frozen_prompt_is_byte_stable_inside_window() {
        let entries: Vec<Entry> = (0..700).map(|i| entry(i, 5)).collect();
        let napped = summarized(&entries);
        for w in [1, 32, 199] {
            let h = 400 / w * w;
            let before_nap = summarized(&entries[..h / 2]);
            let first = frozen_block(&entries[..h], &before_nap, 200, w, None);
            assert!(!first.lines.is_empty(), "w={w}");
            let prompt = crate::chat::turn_system_prompt(&first.lines);
            let nap_at = h + w / 2;
            let mut pinned = first;
            for t in h..h + w {
                let tree = if t < nap_at { &before_nap } else { &napped };
                pinned = frozen_block(&entries[..t], tree, 200, w, Some(pinned));
                assert_eq!(pinned.end, h, "w={w} t={t}");
                assert_eq!(
                    crate::chat::turn_system_prompt(&pinned.lines),
                    prompt,
                    "w={w} t={t}"
                );
            }
            let unpinned = frozen_block(&entries[..nap_at], &napped, 200, w, None);
            assert_ne!(
                crate::chat::turn_system_prompt(&unpinned.lines),
                prompt,
                "w={w}"
            );
        }
    }
}
