use super::*;
use crate::chat_memory::fixtures::{entry, n, texted};
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

fn with_placeholder(mut entries: Vec<Entry>, at: usize) -> Vec<Entry> {
    entries[at] = Entry::corrupt();
    entries
}

#[test]
fn pending_never_offers_a_node_covering_a_placeholder() {
    let entries = with_placeholder((0..8).map(|i| entry(i, 600)).collect(), 5);
    let offered = pending(&entries, &Tree::new());
    assert!(offered.iter().all(|node| !(node.lo..node.hi).contains(&5)));
    for node in [n(5, 6), n(4, 6), n(4, 8), n(0, 8)] {
        assert!(!offered.contains(&node), "{}", node.id());
    }
    for node in [n(0, 1), n(7, 8), n(0, 2), n(6, 8), n(0, 4)] {
        assert!(offered.contains(&node), "{}", node.id());
    }
    assert_eq!(offered.len(), 11);
}

#[test]
fn pending_is_unchanged_when_no_entry_is_a_placeholder() {
    let entries: Vec<Entry> = (0..5).map(|i| entry(i, 5)).collect();
    assert_eq!(pending(&entries, &Tree::new()), [n(0, 2), n(2, 4), n(0, 4)]);
}

#[test]
fn the_view_falls_back_to_children_for_a_skipped_range() {
    let entries = with_placeholder(short(8), 5);
    let tree = summarized(&entries);
    assert!(!tree.contains_key(&n(4, 6)) && !tree.contains_key(&n(0, 8)));
    assert!(tree.contains_key(&n(0, 4)) && tree.contains_key(&n(6, 8)));
    let lines = view(&entries, &tree, 4);
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert!(lines[0].starts_with("#0-3 ") && lines[1].starts_with("#4 "));
    assert_eq!(lines[2], "[#5: unreadable log line, skipped]");
    assert!(lines[3].starts_with("#6-7 "));
}

fn marked(count: usize, at: &[usize]) -> Vec<Entry> {
    at.iter()
        .fold(short(count), |entries, i| with_placeholder(entries, *i))
}

#[test]
fn the_view_collapses_a_run_of_placeholders_into_one_notice() {
    let entries = marked(8, &[5, 6]);
    let lines = view(&entries, &summarized(&entries), 8);
    assert_eq!(lines.len(), 7, "{lines:?}");
    assert!(lines[4].starts_with("#4 "));
    assert_eq!(lines[5], "[#5-6: unreadable log lines, skipped]");
    assert!(lines[6].starts_with("#7 "));
    let lines = view(&marked(8, &[5, 6, 7]), &Tree::new(), 8);
    assert_eq!(lines.len(), 6, "{lines:?}");
    assert!(lines[4].starts_with("#4 "));
    assert_eq!(lines[5], "[#5-7: unreadable log lines, skipped]");
}

#[test]
fn the_view_shows_a_placeholder_at_the_start_as_a_notice() {
    let entries = marked(4, &[0, 1]);
    let lines = view(&entries, &Tree::new(), 4);
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert_eq!(lines[0], "[#0-1: unreadable log lines, skipped]");
    assert!(lines[1].starts_with("#2 ") && lines[2].starts_with("#3 "));
}

#[test]
fn the_tail_collapses_placeholders_and_keeps_other_ids() {
    let entries = marked(6, &[0, 3, 4]);
    for budget in [0, 100] {
        let lines = tail_lines(&entries, &Tree::new(), 0, budget);
        assert_eq!(lines.len(), 5, "{lines:?}");
        assert_eq!(lines[0], "[#0: unreadable log line, skipped]");
        assert!(lines[1].starts_with("#1 ") && lines[2].starts_with("#2 "));
        assert_eq!(lines[3], "[#3-4: unreadable log lines, skipped]");
        assert!(lines[4].starts_with("#5 "));
    }
    let at_end = tail_lines(&marked(3, &[2]), &Tree::new(), 1, 100);
    assert_eq!(at_end.len(), 2, "{at_end:?}");
    assert!(at_end[0].starts_with("#1 "));
    assert_eq!(at_end[1], "[#2: unreadable log line, skipped]");
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
    let lines = view(&entries, &tree, 3);
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
fn tail_with_zero_budget_settles_every_message() {
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
        let before = view(&entries[..count], &tree, VIEW_BUDGET);
        let after = view(&entries, &tree, VIEW_BUDGET);
        let same = |k: usize| before.iter().take(k).eq(after.iter().take(k));
        assert_eq!(same(1), first_same, "first line at {count}");
        assert_eq!(same(10), ten_same, "first 10 lines at {count}");
    }
}

#[test]
fn split_view_covers_every_entry_once() {
    let entries: Vec<Entry> = (0..300).map(|i| entry(i, 5)).collect();
    let tree = summarized(&entries);
    for recent in [0, 30, 300, 100_000] {
        let start = verbatim_start(&entries, 0, recent);
        let split = split_view(&entries, &tree, 200, recent);
        assert_eq!(
            split.older,
            view(&entries[..start], &tree, 200),
            "r={recent}"
        );
        assert_eq!(split.tail.len(), 300 - start, "r={recent}");
        assert!(split.older.len() <= 200, "r={recent}");
    }
}

#[test]
fn split_view_tail_respects_token_budget() {
    let entries: Vec<Entry> = (0..50).map(|i| entry(i, 30)).collect();
    let split = split_view(&entries, &Tree::new(), 200, 100);
    let used: usize = split.tail.len() * estimate_tokens(&entries[0].text);
    assert!(!split.tail.is_empty() && used <= 100);
    assert_eq!(split.older.len(), 50 - split.tail.len());
}

#[test]
fn split_view_of_empty_store_is_empty() {
    assert_eq!(
        split_view(&[], &Tree::new(), 200, 8192),
        ViewSplit::default()
    );
}

fn line_node(line: &str) -> Node {
    Node::parse(line[1..].split(' ').next().unwrap()).unwrap()
}

fn is_marker(line: &str) -> bool {
    line.contains(UNAVAILABLE_MARK)
}

fn ids(lines: &[String]) -> Vec<String> {
    lines.iter().map(|l| line_node(l).id()).collect()
}

fn assert_view_invariants(lines: &[String], t: usize, budget: usize) {
    let cover_len = cover(t, budget).len();
    assert!(lines.len() <= budget.max(cover_len), "t={t} b={budget}");
    assert_eq!(lines_cover(lines), t, "t={t} b={budget}");
    let mut next = 0;
    for line in lines {
        let node = line_node(line);
        assert_eq!(node.lo, next, "t={t} b={budget}");
        next = node.hi;
    }
    assert_eq!(next, t, "t={t} b={budget}");
}

fn short(count: usize) -> Vec<Entry> {
    (0..count).map(|i| entry(i, 5)).collect()
}

fn tree_of(nodes: &[Node]) -> Tree {
    nodes.iter().map(|n| (*n, format!("s{}", n.id()))).collect()
}

#[test]
fn view_merges_an_older_summarized_pair_to_expand_a_missing_parent() {
    let entries = short(9);
    let tree = tree_of(&[n(0, 2), n(2, 4), n(0, 4)]);
    let lines = view(&entries, &tree, 6);
    assert_eq!(ids(&lines), ["0-3", "4", "5", "6", "7", "8"]);
    assert_eq!(lines[0], "#0-3 s0-3");
    assert!(lines.iter().all(|line| !is_marker(line)));
    assert_view_invariants(&lines, 9, 6);
}

#[test]
fn view_with_nothing_mergeable_shows_a_marker_for_a_missing_parent() {
    let entries = short(9);
    let lines = view(&entries, &Tree::new(), 4);
    assert_eq!(ids(&lines), ["0-3", "4-5", "6-7", "8"]);
    assert_eq!(lines[0], "#0-3 [not summarized] zoom 0-3 for its parts");
    assert_eq!(lines.iter().filter(|l| is_marker(l)).count(), 3);
    assert_view_invariants(&lines, 9, 4);
}

#[test]
fn view_shows_a_raw_excerpt_for_a_missing_long_leaf_summary() {
    let entries: Vec<Entry> = (0..3).map(|i| entry(i, 600)).collect();
    let lines = view(&entries, &Tree::new(), 3);
    assert!(lines.iter().all(|line| line.contains(FALLBACK_MARK)));
    assert_view_invariants(&lines, 3, 3);
}

#[test]
fn view_expands_a_missing_node_with_one_summarized_half() {
    let entries = short(9);
    let tree = tree_of(&[n(0, 2), n(6, 8)]);
    let lines = view(&entries, &tree, 5);
    assert_eq!(ids(&lines), ["0-3", "4", "5", "6-7", "8"]);
    assert!(is_marker(&lines[0]));
    assert_eq!(lines[3], "#6-7 s6-7");
    assert_view_invariants(&lines, 9, 5);

    let full = tree_of(&[n(0, 2), n(2, 4), n(4, 6), n(6, 8)]);
    let lines = view(&entries, &full, 5);
    assert_eq!(ids(&lines), ["0-1", "2-3", "4-5", "6-7", "8"]);
    assert!(lines.iter().all(|line| !is_marker(line)));
}

#[test]
fn view_of_a_fully_summarized_tree_is_one_line_per_cover_node() {
    let entries = mixed(40);
    let lines = view(&entries, &summarized(&entries), 10);
    assert_eq!(lines.len(), cover(40, 10).len());
    assert!(lines
        .iter()
        .all(|line| !is_marker(line) && !line.contains(FALLBACK_MARK)));
    assert_view_invariants(&lines, 40, 10);
}

#[test]
fn view_keeps_invariants_when_every_kth_summary_is_dropped() {
    let entries = mixed(100);
    let full = summarized(&entries);
    for k in 2..=5 {
        let tree = drop_every(&full, k);
        for budget in [1, 5, 10, 40, 200] {
            let lines = view(&entries, &tree, budget);
            assert_view_invariants(&lines, 100, budget);
            assert_markers_older_than_splits(&lines, 100, budget);
        }
    }
}

#[test]
fn view_under_a_tight_budget_marks_old_ranges_and_keeps_new_messages() {
    let entries = short(16);
    let lines = view(&entries, &Tree::new(), 3);
    assert_eq!(ids(&lines), ["0-7", "8-11", "12-13", "14", "15"]);
    assert!(lines[..3].iter().all(|line| is_marker(line)));
    assert!(lines[3].ends_with("m14 xxxxx") && lines[4].ends_with("m15 xxxxx"));
}

#[test]
fn view_splits_the_newest_missing_node_first() {
    let entries = short(10);
    let tree = tree_of(&[n(8, 10)]);
    let lines = view(&entries, &tree, 5);
    assert_eq!(ids(&lines), ["0-3", "4-5", "6", "7", "8-9"]);
    assert!(is_marker(&lines[0]) && is_marker(&lines[1]));
    assert!(!is_marker(&lines[2]) && !is_marker(&lines[3]));
}

fn assert_markers_older_than_splits(lines: &[String], t: usize, budget: usize) -> usize {
    let nodes: Vec<Node> = lines.iter().map(|l| line_node(l)).collect();
    let inside = |o: &Node, c: &Node| o.size() < c.size() && c.lo <= o.lo && o.hi <= c.hi;
    let splits: Vec<Node> = cover(t, budget)
        .into_iter()
        .filter(|c| nodes.iter().any(|o| inside(o, c)))
        .collect();
    for (line, marker) in lines.iter().zip(&nodes) {
        if is_marker(line) && !splits.iter().any(|c| inside(marker, c)) {
            assert!(
                splits.iter().all(|c| marker.hi <= c.lo),
                "marker {} newer than a split, t={t} b={budget}",
                marker.id()
            );
        }
    }
    splits.len()
}

fn drop_every(full: &Tree, k: usize) -> Tree {
    let mut keys: Vec<Node> = full.keys().copied().collect();
    keys.sort();
    let mut tree = full.clone();
    for node in keys.iter().step_by(k) {
        tree.remove(node);
    }
    tree
}

#[test]
fn view_keeps_markers_older_than_every_split_under_tight_budgets() {
    let entries = mixed(100);
    let tree = drop_every(&summarized(&entries), 2);
    for budget in [1, 5] {
        let lines = view(&entries, &tree, budget);
        assert!(lines.iter().any(|line| is_marker(line)), "b={budget}");
        assert!(assert_markers_older_than_splits(&lines, 100, budget) > 0);
    }
}

#[test]
fn cover_partitions_the_range_into_blocks_and_honours_the_budget() {
    for t in 1..=64usize {
        for budget in 1..=70usize {
            let nodes = cover(t, budget);
            assert_eq!(nodes[0].lo, 0, "t={t} b={budget}");
            assert_eq!(nodes[nodes.len() - 1].hi, t, "t={t} b={budget}");
            assert!(
                nodes.windows(2).all(|pair| pair[0].hi == pair[1].lo),
                "t={t} b={budget}"
            );
            assert!(
                nodes.iter().all(|node| Node::new(node.lo, node.hi).is_ok()),
                "t={t} b={budget}"
            );
            assert!(nodes.len() >= budget.min(t), "t={t} b={budget}");
        }
    }
}

#[test]
fn an_entry_is_long_only_above_the_summary_size() {
    let at = |len: usize| texted(1, |_| "a".repeat(len)).remove(0);
    assert!(!is_long(&at(SUMMARY_BYTES)));
    assert!(is_long(&at(SUMMARY_BYTES + 1)));
}
