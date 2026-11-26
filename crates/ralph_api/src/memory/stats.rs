use ralph_core::chat_memory::{root_view, Entry, Node, Tree};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NodeStats {
    pub depth: usize,
    pub children: usize,
    pub blocks: usize,
    pub summarised: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TreeStats {
    pub entries: usize,
    pub blocks: usize,
    pub summarised: usize,
    pub last_at: Option<String>,
}

pub(crate) fn node_stats(node: &Node, tree: &Tree) -> NodeStats {
    let size = node.size();
    let blocks = size.saturating_sub(1);
    let summarised = tree
        .keys()
        .filter(|key| key.size() >= 2 && key.lo >= node.lo && key.hi <= node.hi)
        .count();
    NodeStats {
        depth: size.checked_ilog2().unwrap_or(0) as usize,
        children: if size >= 2 { 2 } else { 0 },
        blocks,
        summarised,
    }
}

pub(crate) fn is_folded(stats: &NodeStats) -> bool {
    stats.blocks >= 1 && stats.summarised == 0
}

pub(crate) fn tree_stats(entries: &[Entry], tree: &Tree) -> TreeStats {
    let roots = root_view(entries.len(), tree);
    let (blocks, summarised) = roots
        .iter()
        .map(|view| node_stats(&view.node, tree))
        .fold((0, 0), |(blocks, summarised), stats| {
            (blocks + stats.blocks, summarised + stats.summarised)
        });
    TreeStats {
        entries: entries.len(),
        blocks,
        summarised,
        last_at: entries.last().map(|entry| entry.at.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::chat_memory::Role;

    fn node(lo: usize, hi: usize) -> Node {
        Node::new(lo, hi).unwrap()
    }

    fn tree(nodes: &[Node]) -> Tree {
        nodes.iter().map(|n| (*n, "s".to_string())).collect()
    }

    fn entry(at: &str) -> Entry {
        Entry {
            role: Role::User,
            at: at.to_string(),
            text: "t".to_string(),
            corrupt: false,
        }
    }

    #[test]
    fn stats_for_a_size_8_node_with_two_summaries() {
        let tree = tree(&[node(0, 8), node(0, 4), node(8, 12)]);
        assert_eq!(
            node_stats(&node(0, 8), &tree),
            NodeStats {
                depth: 3,
                children: 2,
                blocks: 7,
                summarised: 2,
            }
        );
    }

    #[test]
    fn a_node_with_no_summary_counts_all_its_blocks_as_unsummarised() {
        assert_eq!(
            node_stats(&node(0, 4), &Tree::new()),
            NodeStats {
                depth: 2,
                children: 2,
                blocks: 3,
                summarised: 0,
            }
        );
    }

    #[test]
    fn a_size_1_node_has_no_blocks() {
        assert_eq!(
            node_stats(&node(5, 6), &Tree::new()),
            NodeStats {
                depth: 0,
                children: 0,
                blocks: 0,
                summarised: 0,
            }
        );
    }

    #[test]
    fn a_block_with_no_summary_below_is_folded() {
        assert!(is_folded(&node_stats(&node(0, 4), &Tree::new())));
    }

    #[test]
    fn a_block_with_one_summary_below_is_not_folded() {
        assert!(!is_folded(&node_stats(&node(0, 8), &tree(&[node(2, 4)]))));
    }

    #[test]
    fn a_block_with_its_own_summary_only_is_not_folded() {
        assert!(!is_folded(&node_stats(&node(0, 4), &tree(&[node(0, 4)]))));
    }

    #[test]
    fn a_size_1_node_is_never_folded() {
        assert!(!is_folded(&node_stats(&node(5, 6), &Tree::new())));
    }

    #[test]
    fn tree_stats_sum_the_root_nodes() {
        let entries: Vec<Entry> = (0..11).map(|i| entry(&format!("t{i}"))).collect();
        let tree = tree(&[node(0, 8), node(8, 10)]);
        assert_eq!(
            tree_stats(&entries, &tree),
            TreeStats {
                entries: 11,
                blocks: 8,
                summarised: 2,
                last_at: Some("t10".to_string()),
            }
        );
    }

    #[test]
    fn tree_stats_of_an_empty_log() {
        assert_eq!(
            tree_stats(&[], &Tree::new()),
            TreeStats {
                entries: 0,
                blocks: 0,
                summarised: 0,
                last_at: None,
            }
        );
    }
}
