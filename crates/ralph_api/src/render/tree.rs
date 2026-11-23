use crate::render::{empty, escape};

pub enum TreeNode<'a> {
    Branch {
        label: &'a str,
        meta: Option<&'a str>,
        expanded: bool,
        children: Vec<TreeNode<'a>>,
    },
    Leaf {
        label: &'a str,
        meta: Option<&'a str>,
    },
    Row(String),
}

fn meta_html(meta: Option<&str>) -> String {
    meta.map_or_else(String::new, |text| {
        format!("<span class=\"meta\">{}</span>", escape(text))
    })
}

fn item(node: &TreeNode, level: usize) -> String {
    match node {
        TreeNode::Branch {
            label,
            meta,
            expanded,
            children,
        } => {
            let group = if children.is_empty() {
                String::new()
            } else {
                format!("<ul role=\"group\">{}</ul>", items(children, level + 1))
            };
            format!(
                "<li role=\"treeitem\" aria-expanded=\"{expanded}\" aria-level=\"{level}\"><button type=\"button\">{}</button>{}{group}</li>",
                escape(label),
                meta_html(*meta)
            )
        }
        TreeNode::Leaf { label, meta } => format!(
            "<li role=\"treeitem\" aria-level=\"{level}\"><span class=\"label\">{}</span>{}</li>",
            escape(label),
            meta_html(*meta)
        ),
        TreeNode::Row(html) => format!("<li role=\"none\">{html}</li>"),
    }
}

fn items(nodes: &[TreeNode], level: usize) -> String {
    nodes.iter().map(|node| item(node, level)).collect()
}

pub fn tree(nodes: &[TreeNode]) -> String {
    if nodes.is_empty() {
        return format!("<r-tree>{}</r-tree>", empty("Nothing to show"));
    }
    format!(
        "<r-tree><ul role=\"tree\">{}</ul></r-tree>",
        items(nodes, 1)
    )
}

#[cfg(test)]
mod tests {
    use super::{tree, TreeNode};
    use crate::render::golden::{assert_golden, hostile_text, long_text};

    fn leaf<'a>(label: &'a str) -> TreeNode<'a> {
        TreeNode::Leaf { label, meta: None }
    }

    #[test]
    fn no_nodes_shows_the_empty_state() {
        assert_golden("tree_empty", &tree(&[]));
    }

    #[test]
    fn one_expanded_branch_with_a_leaf_and_meta() {
        let nodes = [TreeNode::Branch {
            label: "Summary",
            meta: Some("3 turns"),
            expanded: true,
            children: vec![TreeNode::Leaf {
                label: "Decision",
                meta: Some("ok"),
            }],
        }];
        assert_golden("tree_sparse", &tree(&nodes));
    }

    #[test]
    fn collapsed_branch_keeps_its_children_and_says_so() {
        let nodes = [TreeNode::Branch {
            label: "Old",
            meta: None,
            expanded: false,
            children: vec![leaf("hidden")],
        }];
        let html = tree(&nodes);
        assert!(html.contains("aria-expanded=\"false\""));
        assert!(html.contains("hidden"));
        assert_golden("tree_collapsed", &html);
    }

    #[test]
    fn levels_count_from_one_and_grow_with_depth() {
        let nodes = [TreeNode::Branch {
            label: "a",
            meta: None,
            expanded: true,
            children: vec![TreeNode::Branch {
                label: "b",
                meta: None,
                expanded: true,
                children: vec![leaf("c")],
            }],
        }];
        let html = tree(&nodes);
        for level in 1..=3 {
            assert!(html.contains(&format!("aria-level=\"{level}\"")), "{level}");
        }
        assert!(!html.contains("aria-level=\"4\""));
    }

    #[test]
    fn a_branch_without_children_has_no_group() {
        let nodes = [TreeNode::Branch {
            label: "a",
            meta: None,
            expanded: true,
            children: vec![],
        }];
        assert!(!tree(&nodes).contains("role=\"group\""));
    }

    #[test]
    fn a_row_node_is_not_a_treeitem() {
        let nodes = [TreeNode::Row("<r-error role=\"alert\">x</r-error>".into())];
        let html = tree(&nodes);
        assert!(html.contains("<li role=\"none\"><r-error"));
        assert!(!html.contains("treeitem"));
    }

    #[test]
    fn hostile_text_is_escaped() {
        let nodes = [TreeNode::Branch {
            label: hostile_text(),
            meta: Some(hostile_text()),
            expanded: true,
            children: vec![leaf(hostile_text())],
        }];
        let html = tree(&nodes);
        assert!(!html.contains("<script"));
        assert_golden("tree_hostile", &html);
    }

    #[test]
    fn very_long_text() {
        let long = long_text();
        let nodes = [TreeNode::Branch {
            label: &long,
            meta: Some(&long),
            expanded: true,
            children: vec![leaf(&long)],
        }];
        assert_golden("tree_very_long", &tree(&nodes));
    }
}
