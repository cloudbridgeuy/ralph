use crate::chat_memory::{cover_candidates, Node, Tree};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ancestor<'a> {
    pub node: Node,
    pub depth: u32,
    pub summary: &'a str,
}

pub fn ancestors(id: usize, total: usize, tree: &Tree) -> Vec<Ancestor<'_>> {
    cover_candidates(id, total)
        .into_iter()
        .filter_map(|node| {
            tree.get(&node).map(|summary| Ancestor {
                node,
                depth: node.size().trailing_zeros(),
                summary: summary.as_str(),
            })
        })
        .collect()
}
