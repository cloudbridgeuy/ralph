use super::{CallMetrics, Node, RangeRequest, Tree};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub text: String,
    pub hits: usize,
    pub truncated: Option<bool>,
    pub oldest: Option<usize>,
    pub top: Option<usize>,
}

fn count(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

pub fn cover_candidates(id: usize, total: usize) -> Vec<Node> {
    (0..usize::BITS)
        .map(|shift| 1_usize << shift)
        .take_while(|size| *size / 2 < total.max(1))
        .map(|size| {
            let lo = id - id % size;
            Node { lo, hi: lo + size }
        })
        .collect()
}

pub fn cover_size(tree: &Tree, id: usize, total: usize) -> u64 {
    cover_candidates(id, total)
        .into_iter()
        .filter(|node| tree.contains_key(node))
        .map(|node| count(node.size()))
        .max()
        .unwrap_or(1)
}

pub fn zoom_metrics(node: Node) -> CallMetrics {
    CallMetrics {
        node_size: Some(count(node.size())),
        depth: Some(u64::from(node.size().trailing_zeros())),
        ..CallMetrics::default()
    }
}

pub fn read_metrics(request: &RangeRequest) -> CallMetrics {
    CallMetrics {
        node_size: Some(count(
            request.to.saturating_sub(request.from).saturating_add(1),
        )),
        ..CallMetrics::default()
    }
}

pub fn found_metrics(found: &Found, top_cover_size: Option<u64>) -> CallMetrics {
    CallMetrics {
        hits: Some(count(found.hits)),
        truncated: found.truncated,
        oldest_hit: found.oldest.map(count),
        top_cover_size: found.top.and(top_cover_size),
        ..CallMetrics::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_memory::fixtures::n;
    use crate::chat_memory::Mode;

    #[test]
    fn zoom_metrics_report_size_and_depth() {
        let leaf = zoom_metrics(n(5, 6));
        assert_eq!((leaf.node_size, leaf.depth), (Some(1), Some(0)));
        let block = zoom_metrics(n(0, 8));
        assert_eq!((block.node_size, block.depth), (Some(8), Some(3)));
        assert_eq!(block.hits, None);
    }

    #[test]
    fn read_metrics_count_the_requested_entries() {
        let request = RangeRequest {
            from: 2,
            to: 5,
            mode: Mode::Raw,
            budget: 100,
            cursor: None,
        };
        assert_eq!(read_metrics(&request).node_size, Some(4));
        assert_eq!(read_metrics(&request).depth, None);
    }

    #[test]
    fn cover_size_picks_the_largest_summarized_block() {
        let mut tree = Tree::new();
        assert_eq!(cover_size(&tree, 5, 9), 1);
        tree.insert(n(4, 6), "pair".into());
        assert_eq!(cover_size(&tree, 5, 9), 2);
        tree.insert(n(0, 8), "block".into());
        assert_eq!(cover_size(&tree, 5, 9), 8);
        assert_eq!(cover_size(&tree, 8, 9), 1);
    }

    #[test]
    fn found_metrics_carry_the_lookup_facts() {
        let found = Found {
            text: String::new(),
            hits: 60,
            truncated: Some(true),
            oldest: Some(3),
            top: Some(9),
        };
        let metrics = found_metrics(&found, Some(4));
        assert_eq!(metrics.hits, Some(60));
        assert_eq!(metrics.truncated, Some(true));
        assert_eq!(metrics.oldest_hit, Some(3));
        assert_eq!(metrics.top_cover_size, Some(4));
        assert_eq!(metrics.in_view, None);
        let empty = Found {
            text: String::new(),
            hits: 0,
            truncated: Some(false),
            oldest: None,
            top: None,
        };
        let metrics = found_metrics(&empty, Some(4));
        assert_eq!((metrics.hits, metrics.top_cover_size), (Some(0), None));
    }
}
