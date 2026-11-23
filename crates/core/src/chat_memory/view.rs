use super::{Entry, Node, Role, Tree};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeView {
    pub node: Node,
    pub summary: Option<String>,
    pub is_leaf: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryView {
    pub i: usize,
    pub role: Role,
    pub at: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZoomChild {
    Block(NodeView),
    Entry(EntryView),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoomView {
    pub node: Node,
    pub summary: Option<String>,
    pub children: Vec<ZoomChild>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub items: Vec<EntryView>,
    pub next_after: Option<usize>,
}

fn node_view(tree: &Tree, node: Node) -> NodeView {
    NodeView {
        node,
        summary: tree.get(&node).cloned(),
        is_leaf: node.size() == 1,
    }
}

fn entry_view(entries: &[Entry], i: usize) -> Option<EntryView> {
    entries.get(i).map(|entry| EntryView {
        i,
        role: entry.role,
        at: entry.at.clone(),
        text: entry.text.clone(),
    })
}

pub fn root_view(total: usize, tree: &Tree) -> Vec<NodeView> {
    let mut lo = 0;
    (0..usize::BITS)
        .rev()
        .map(|bit| 1usize << bit)
        .filter(|size| total & size != 0)
        .map(|size| {
            let node = Node { lo, hi: lo + size };
            lo += size;
            node_view(tree, node)
        })
        .collect()
}

pub fn zoom_view(entries: &[Entry], tree: &Tree, node: Node) -> Result<ZoomView, String> {
    let total = entries.len();
    if node.hi > total {
        return Err(format!(
            "#{} is beyond the chat: it holds {total} entries.",
            node.id()
        ));
    }
    let child = |half: Node| {
        if half.size() == 1 {
            entry_view(entries, half.lo).map(ZoomChild::Entry)
        } else {
            Some(ZoomChild::Block(node_view(tree, half)))
        }
    };
    let children = if node.size() == 1 {
        child(node).into_iter().collect()
    } else {
        let (left, right) = node.halves();
        [left, right].into_iter().filter_map(child).collect()
    };
    Ok(ZoomView {
        node,
        summary: tree.get(&node).cloned(),
        children,
    })
}

pub fn page(entries: &[Entry], after: usize, limit: usize) -> Page {
    let start = after.min(entries.len());
    let end = start.saturating_add(limit).min(entries.len());
    Page {
        items: (start..end)
            .filter_map(|i| entry_view(entries, i))
            .collect(),
        next_after: (limit > 0 && end < entries.len()).then_some(end),
    }
}

pub fn tree_within(tree: &Tree, total: usize) -> Tree {
    tree.iter()
        .filter(|(node, _)| node.hi <= total)
        .map(|(node, summary)| (*node, summary.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_memory::fixtures::{entry, n, texted};
    use crate::chat_memory::{read_range, zoom, Mode, RangeRequest, Read, Window};

    fn ids(views: &[NodeView]) -> Vec<String> {
        views.iter().map(|v| v.node.id()).collect()
    }

    fn tree_of(nodes: &[(usize, usize, &str)]) -> Tree {
        nodes
            .iter()
            .map(|(lo, hi, s)| (n(*lo, *hi), s.to_string()))
            .collect()
    }

    #[test]
    fn root_decomposition_by_total() {
        let none = Tree::new();
        let cases: [(usize, &[&str]); 7] = [
            (0, &[]),
            (1, &["0"]),
            (2, &["0-1"]),
            (3, &["0-1", "2"]),
            (7, &["0-3", "4-5", "6"]),
            (8, &["0-7"]),
            (11, &["0-7", "8-9", "10"]),
        ];
        for (total, want) in cases {
            assert_eq!(ids(&root_view(total, &none)), want, "total {total}");
        }
    }

    #[test]
    fn root_view_carries_summaries_and_leaf_flag() {
        let tree = tree_of(&[(0, 8, "big"), (8, 10, "pair")]);
        let view = root_view(11, &tree);
        assert_eq!(view[0].summary.as_deref(), Some("big"));
        assert_eq!(view[1].summary.as_deref(), Some("pair"));
        assert_eq!(view[2].summary, None);
        assert_eq!(
            view.iter().map(|v| v.is_leaf).collect::<Vec<_>>(),
            [false, false, true]
        );
    }

    #[test]
    fn zoom_with_missing_summaries() {
        let entries = texted(8, |i| format!("e{i}"));
        let tree = tree_of(&[(0, 4, "left")]);
        let view = zoom_view(&entries, &tree, n(0, 8)).unwrap();
        assert_eq!(view.summary, None);
        assert_eq!(
            view.children,
            vec![
                ZoomChild::Block(NodeView {
                    node: n(0, 4),
                    summary: Some("left".into()),
                    is_leaf: false
                }),
                ZoomChild::Block(NodeView {
                    node: n(4, 8),
                    summary: None,
                    is_leaf: false
                }),
            ]
        );
    }

    #[test]
    fn zoom_of_leaf_pair_lists_entries() {
        let entries = texted(4, |i| format!("e{i}"));
        let tree = tree_of(&[(2, 4, "pair")]);
        let view = zoom_view(&entries, &tree, n(2, 4)).unwrap();
        assert_eq!(view.summary.as_deref(), Some("pair"));
        let got: Vec<_> = view
            .children
            .iter()
            .map(|c| match c {
                ZoomChild::Entry(e) => (e.i, e.text.clone()),
                ZoomChild::Block(_) => panic!("expected entry"),
            })
            .collect();
        assert_eq!(got, [(2, "e2".to_string()), (3, "e3".to_string())]);
    }

    #[test]
    fn zoom_of_leaf_returns_the_entry() {
        let entries = texted(3, |i| format!("e{i}"));
        let view = zoom_view(&entries, &Tree::new(), n(1, 2)).unwrap();
        assert_eq!(view.children.len(), 1);
        assert!(matches!(&view.children[0], ZoomChild::Entry(e) if e.i == 1));
    }

    #[test]
    fn zoom_beyond_total_errors() {
        let entries = texted(6, |i| format!("e{i}"));
        assert!(zoom_view(&entries, &Tree::new(), n(0, 8)).is_err());
        assert!(zoom_view(&entries, &Tree::new(), n(8, 9)).is_err());
        assert!(zoom_view(&entries, &Tree::new(), n(6, 8)).is_err());
        assert!(zoom_view(&entries, &Tree::new(), n(4, 6)).is_ok());
    }

    #[test]
    fn page_boundaries() {
        let entries = texted(5, |i| format!("e{i}"));
        let first = page(&entries, 0, 2);
        assert_eq!(first.items.iter().map(|e| e.i).collect::<Vec<_>>(), [0, 1]);
        assert_eq!(first.next_after, Some(2));
        let last = page(&entries, 2, 3);
        assert_eq!(
            last.items.iter().map(|e| e.i).collect::<Vec<_>>(),
            [2, 3, 4]
        );
        assert_eq!(last.next_after, None);
        let all = page(&entries, 0, 99);
        assert_eq!(all.items.len(), 5);
        assert_eq!(all.next_after, None);
        let past = page(&entries, 5, 2);
        assert!(past.items.is_empty());
        assert_eq!(past.next_after, None);
        let beyond = page(&entries, 50, 2);
        assert!(beyond.items.is_empty());
        assert_eq!(beyond.next_after, None);
        let zero = page(&entries, 1, 0);
        assert!(zero.items.is_empty());
        assert_eq!(zero.next_after, None);
    }

    #[test]
    fn consistency_cut_drops_nodes_past_total() {
        let tree = tree_of(&[(0, 4, "a"), (4, 8, "b"), (0, 8, "c"), (8, 9, "d")]);
        let cut = tree_within(&tree, 8);
        assert_eq!(cut.len(), 3);
        assert!(!cut.contains_key(&n(8, 9)));
        let cut = tree_within(&tree, 5);
        assert_eq!(cut.keys().copied().collect::<Vec<_>>(), [n(0, 4)]);
        assert!(tree_within(&tree, 0).is_empty());
    }

    #[test]
    fn page_matches_read_range_text() {
        let entries: Vec<Entry> = (0..6).map(|i| entry(i, 5)).collect();
        let window = Window::full(&entries);
        let request = RangeRequest {
            from: 0,
            to: 5,
            budget: 4000,
            mode: Mode::Raw,
            cursor: None,
        };
        let read = read_range(&window, &Tree::new(), &request).unwrap();
        let Read::Page(read_page) = read else {
            panic!("expected a page");
        };
        let rendered = read_page.blocks.join("\n");
        for item in page(&entries, 0, 6).items {
            assert!(rendered.contains(&item.text), "missing #{}", item.i);
            assert_eq!(item.text, entries[item.i].text);
        }
    }

    #[test]
    fn zoom_view_matches_zoom_lines() {
        let entries: Vec<Entry> = (0..8).map(|i| entry(i, 5)).collect();
        let tree = tree_of(&[(0, 4, "left summary"), (4, 8, "right summary")]);
        let window = Window::full(&entries);
        for node in [n(0, 8), n(0, 2), n(6, 8), n(3, 4)] {
            let lines = zoom(&window, &tree, node).unwrap();
            let view = zoom_view(&entries, &tree, node).unwrap();
            assert_eq!(lines.len(), view.children.len(), "node {}", node.id());
            for (line, child) in lines.iter().zip(&view.children) {
                match child {
                    ZoomChild::Block(b) => {
                        let want = match &b.summary {
                            Some(s) => format!("#{} {s}", b.node.id()),
                            None => format!("#{} not compressed yet", b.node.id()),
                        };
                        assert_eq!(line, &want);
                    }
                    ZoomChild::Entry(e) => {
                        let want = format!(
                            "#{} {} {} {}",
                            e.i,
                            crate::chat_memory::role_label(e.role, None),
                            e.at,
                            e.text
                        );
                        assert_eq!(line, &want);
                    }
                }
            }
        }
    }

    #[test]
    fn hostile_text_passes_through() {
        let hostile = [
            "<script>alert(1)</script>".to_string(),
            "</r-prose> tail".to_string(),
            "nul\0byte".to_string(),
            "y".repeat(100_000),
            "\u{202e}rtl \u{05d0}\u{05d1} text".to_string(),
        ];
        let entries = texted(hostile.len(), |i| hostile[i].clone());
        let got = page(&entries, 0, 10);
        for (item, want) in got.items.iter().zip(&hostile) {
            assert_eq!(&item.text, want);
        }
        let pair = texted(2, |i| hostile[i].clone());
        let view = zoom_view(&pair, &Tree::new(), n(0, 2)).unwrap();
        for (child, want) in view.children.iter().zip(&hostile) {
            assert!(matches!(child, ZoomChild::Entry(e) if &e.text == want));
        }
    }
}
