use ralph_core::chat_memory::{root_view, Node, Tree};

fn parent(node: &Node) -> Option<Node> {
    let size = node.size().checked_mul(2)?;
    let lo = node.lo - node.lo % size;
    Node::new(lo, lo + size).ok()
}

fn contains(outer: &Node, inner: &Node) -> bool {
    outer.lo <= inner.lo && inner.hi <= outer.hi
}

pub(crate) fn zoom_path(total: usize, node: &Node) -> Vec<String> {
    let Some(root) = root_view(total, &Tree::new())
        .into_iter()
        .map(|view| view.node)
        .find(|root| contains(root, node))
    else {
        return Vec::new();
    };
    let mut path = vec![node.id()];
    let mut current = *node;
    while current != root {
        let Some(up) = parent(&current) else {
            return Vec::new();
        };
        path.push(up.id());
        current = up;
    }
    path.reverse();
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(lo: usize, hi: usize) -> Node {
        Node::new(lo, hi).unwrap()
    }

    #[test]
    fn zoom_path_lists_ancestors_from_the_root() {
        assert_eq!(
            zoom_path(45, &node(0, 2)),
            ["0-31", "0-15", "0-7", "0-3", "0-1"]
        );
    }

    #[test]
    fn zoom_path_of_a_root_node_is_just_itself() {
        assert_eq!(zoom_path(45, &node(0, 32)), ["0-31"]);
        assert_eq!(zoom_path(45, &node(40, 44)), ["40-43"]);
    }

    #[test]
    fn zoom_path_in_a_later_root_starts_at_that_root() {
        assert_eq!(zoom_path(45, &node(44, 45)), ["44"]);
        assert_eq!(zoom_path(45, &node(40, 42)), ["40-43", "40-41"]);
    }

    #[test]
    fn zoom_path_of_a_node_beyond_the_total_is_empty() {
        assert!(zoom_path(3, &node(0, 8)).is_empty());
    }
}
