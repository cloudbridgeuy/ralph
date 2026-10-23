use super::nap::fallback_summary;
use super::notice::{merge_notices, Shown};
use super::{Entry, Node, Tree, SUMMARY_BYTES, UNAVAILABLE_MARK};
use crate::chat::OLDER_VIEW_STEP;

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

fn damaged_prefix(entries: &[Entry]) -> Vec<usize> {
    std::iter::once(0)
        .chain(entries.iter().scan(0, |seen, entry| {
            *seen += usize::from(entry.is_corrupt());
            Some(*seen)
        }))
        .collect()
}

pub fn pending(entries: &[Entry], tree: &Tree) -> Vec<Node> {
    let t = entries.len();
    let damaged = damaged_prefix(entries);
    let intact = |n: &Node| damaged[n.hi] == damaged[n.lo];
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
                .filter(|n| !tree.contains_key(n) && intact(n)),
        );
        size *= 2;
    }
    todo
}

pub(super) fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn entry_line(i: usize, entry: &Entry, body: &str) -> String {
    format!("#{i} {} {} {body}", entry.role.as_str(), entry.at)
}

pub(super) fn leaf_line(i: usize, entry: &Entry, tree: &Tree) -> Option<String> {
    if is_long(entry) {
        tree.get(&Node::leaf(i)).map(|s| entry_line(i, entry, s))
    } else {
        Some(entry_line(i, entry, &collapse(&entry.text)))
    }
}

pub(super) fn block_line(tree: &Tree, node: Node) -> Option<String> {
    tree.get(&node).map(|s| format!("#{} {s}", node.id()))
}

pub(super) fn node_line(entries: &[Entry], tree: &Tree, node: Node) -> Option<String> {
    if node.size() == 1 {
        leaf_line(node.lo, &entries[node.lo], tree)
    } else {
        block_line(tree, node)
    }
}

fn display_line(entries: &[Entry], tree: &Tree, node: Node) -> String {
    node_line(entries, tree, node).unwrap_or_else(|| {
        if node.size() == 1 {
            let entry = &entries[node.lo];
            entry_line(node.lo, entry, &fallback_summary(&entry.text))
        } else {
            let id = node.id();
            format!("#{id} {UNAVAILABLE_MARK}zoom {id} for its parts")
        }
    })
}

fn shown_node(entries: &[Entry], tree: &Tree, node: Node) -> Shown {
    if node.size() == 1 && entries[node.lo].is_corrupt() {
        Shown::Unreadable(node.lo)
    } else {
        Shown::Line(display_line(entries, tree, node))
    }
}

fn split_target(entries: &[Entry], tree: &Tree, nodes: &[Node]) -> Option<Node> {
    nodes
        .iter()
        .filter(|node| node.size() > 1 && node_line(entries, tree, **node).is_none())
        .max_by_key(|node| node.lo)
        .copied()
}

fn mergeable_pair(tree: &Tree, nodes: &[Node], target: Node) -> Option<usize> {
    nodes.windows(2).position(|pair| {
        let (left, right) = (pair[0], pair[1]);
        left.size() == right.size()
            && left.lo % (2 * left.size()) == 0
            && left.hi == right.lo
            && left != target
            && right != target
            && tree.contains_key(&Node {
                lo: left.lo,
                hi: right.hi,
            })
    })
}

fn view(entries: &[Entry], tree: &Tree, budget: usize) -> Vec<String> {
    let mut nodes = cover(entries.len(), budget);
    while let Some(target) = split_target(entries, tree, &nodes) {
        if nodes.len() >= budget {
            let Some(i) = mergeable_pair(tree, &nodes, target) else {
                break;
            };
            let parent = Node {
                lo: nodes[i].lo,
                hi: nodes[i + 1].hi,
            };
            nodes.splice(i..i + 2, [parent]);
        }
        let Some(i) = nodes.iter().position(|node| *node == target) else {
            break;
        };
        let (left, right) = target.halves();
        nodes.splice(i..=i, [left, right]);
    }
    merge_notices(
        nodes
            .into_iter()
            .map(|node| shown_node(entries, tree, node)),
    )
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ViewSplit {
    pub older: Vec<String>,
    pub tail: Vec<String>,
}

pub fn split_view(
    entries: &[Entry],
    tree: &Tree,
    budget: usize,
    recent_tokens: usize,
) -> ViewSplit {
    let cut = stable_cut(verbatim_start(entries, 0, recent_tokens), OLDER_VIEW_STEP);
    ViewSplit {
        older: view(&entries[..cut], tree, budget),
        tail: tail_lines(entries, tree, cut, recent_tokens),
    }
}

pub fn stable_cut(start: usize, step: usize) -> usize {
    start - start.checked_rem(step).unwrap_or(0)
}

pub fn entries_to_shift(entries: &[Entry], recent_tokens: usize) -> usize {
    let start = verbatim_start(entries, 0, recent_tokens);
    stable_cut(start, OLDER_VIEW_STEP) + OLDER_VIEW_STEP - start
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
    let settled = (from..start).map(|i| shown_node(entries, tree, Node::leaf(i)));
    let verbatim = (start..entries.len()).map(|i| match &entries[i] {
        entry if entry.is_corrupt() => Shown::Unreadable(i),
        entry => Shown::Line(entry_line(i, entry, &entry.text)),
    });
    merge_notices(settled.chain(verbatim))
}

#[cfg(test)]
mod tests;
