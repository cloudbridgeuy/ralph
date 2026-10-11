use super::{Entry, Node, Recall, Tree, View, SUMMARY_BYTES};
use regex::RegexBuilder;
use std::collections::VecDeque;

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

pub fn view(entries: &[Entry], tree: &Tree, budget: usize) -> View {
    let mut view = View::default();
    for node in cover(entries.len(), budget) {
        match node_line(entries, tree, node) {
            Some(line) => view.lines.push(line),
            None => view.missing.push(node),
        }
    }
    view
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

pub fn recall(entries: &[Entry], pattern: &str, cap_bytes: usize) -> Result<Recall, String> {
    let re = RegexBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .map_err(|e| format!("bad regex: {e}"))?;
    let mut hits = 0;
    let mut size = 0;
    let mut kept: VecDeque<String> = VecDeque::new();
    for (i, entry) in entries.iter().enumerate() {
        let line = entry_line(i, entry, &entry.text);
        if !re.is_match(&line) {
            continue;
        }
        hits += 1;
        size += line.len() + 1;
        kept.push_back(line);
        while size > cap_bytes {
            match kept.pop_front() {
                Some(old) => size -= old.len() + 1,
                None => break,
            }
        }
    }
    Ok(Recall {
        lines: kept.into(),
        hits,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_memory::fixtures::{entry, n};

    fn mixed(count: usize) -> Vec<Entry> {
        (0..count)
            .map(|i| entry(i, if i.is_multiple_of(3) { 600 } else { 5 }))
            .collect()
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

    #[test]
    fn recall_keeps_newest_within_cap() {
        let entries: Vec<Entry> = (0..5).map(|i| entry(i, 5)).collect();
        let line_len = |i: usize| entry_line(i, &entries[i], &entries[i].text).len() + 1;
        let found = recall(&entries, "XXXXX", line_len(3) + line_len(4)).unwrap();
        assert_eq!(found.hits, 5);
        assert_eq!(found.lines.len(), 2);
        assert!(found.lines[0].starts_with("#3 "));
        assert!(recall(&entries, "(", 10).is_err());
        assert_eq!(recall(&entries, "nomatch", 100).unwrap().hits, 0);
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
}
