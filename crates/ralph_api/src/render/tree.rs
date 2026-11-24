use crate::render::{empty, escape};

#[derive(Default)]
pub struct Cells<'a> {
    pub count: Option<usize>,
    pub summary: Option<&'a str>,
}

pub enum TreeNode<'a> {
    Branch {
        label: &'a str,
        cells: Cells<'a>,
        expanded: bool,
        children: Vec<TreeNode<'a>>,
    },
    Leaf {
        label: &'a str,
        cells: Cells<'a>,
    },
    Row(String),
}

const CHEVRON: &str = "<svg class=\"chev\" viewBox=\"0 0 8 8\" aria-hidden=\"true\"><path d=\"M2.5 1l3.5 3-3.5 3\"/></svg>";

fn guides(level: usize) -> String {
    "<span class=\"g\"></span>".repeat(level.saturating_sub(1))
}

fn cells_html(cells: &Cells) -> String {
    let count = cells.count.map(|n| n.to_string()).unwrap_or_default();
    let summary = cells.summary.map(escape).unwrap_or_default();
    format!("<span class=\"n\">{count}</span><span class=\"sum\">{summary}</span>")
}

fn item(node: &TreeNode, level: usize) -> String {
    match node {
        TreeNode::Branch {
            label,
            cells,
            expanded,
            children,
        } => {
            let group = if children.is_empty() {
                String::new()
            } else {
                format!("<ul role=\"group\">{}</ul>", items(children, level + 1))
            };
            format!(
                "<li role=\"treeitem\" aria-expanded=\"{expanded}\" aria-level=\"{level}\"><button type=\"button\">{}{CHEVRON}<span class=\"id\">{}</span></button>{}{group}</li>",
                guides(level),
                escape(label),
                cells_html(cells)
            )
        }
        TreeNode::Leaf { label, cells } => format!(
            "<li role=\"treeitem\" aria-level=\"{level}\"><span class=\"label\">{}<span class=\"chev\"></span><span class=\"id\">{}</span></span>{}</li>",
            guides(level),
            escape(label),
            cells_html(cells)
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
    use super::{tree, Cells, TreeNode};
    use crate::render::golden::{assert_golden, hostile_text, long_text};

    fn leaf<'a>(label: &'a str) -> TreeNode<'a> {
        TreeNode::Leaf {
            label,
            cells: Cells::default(),
        }
    }

    fn branch<'a>(label: &'a str, expanded: bool, children: Vec<TreeNode<'a>>) -> TreeNode<'a> {
        TreeNode::Branch {
            label,
            cells: Cells::default(),
            expanded,
            children,
        }
    }

    #[test]
    fn no_nodes_shows_the_empty_state() {
        assert_golden("tree_empty", &tree(&[]));
    }

    #[test]
    fn one_expanded_branch_with_a_leaf_and_cells() {
        let nodes = [TreeNode::Branch {
            label: "0-31",
            cells: Cells {
                count: Some(32),
                summary: Some("Early work"),
            },
            expanded: true,
            children: vec![TreeNode::Leaf {
                label: "#4 user",
                cells: Cells {
                    count: None,
                    summary: Some("hello"),
                },
            }],
        }];
        assert_golden("tree_sparse", &tree(&nodes));
    }

    #[test]
    fn a_row_has_label_count_and_summary_cells_in_order() {
        let nodes = [TreeNode::Branch {
            label: "0-31",
            cells: Cells {
                count: Some(32),
                summary: Some("Early work"),
            },
            expanded: true,
            children: vec![],
        }];
        let html = tree(&nodes);
        let at = |needle: &str| html.find(needle).unwrap_or_else(|| panic!("{needle}"));
        assert!(at("<span class=\"id\">0-31</span>") < at("<span class=\"n\">32</span>"));
        assert!(at("<span class=\"n\">32</span>") < at("<span class=\"sum\">Early work</span>"));
    }

    #[test]
    fn a_missing_cell_still_leaves_an_empty_span_so_columns_hold() {
        let html = tree(&[leaf("a")]);
        assert!(html.contains("<span class=\"n\"></span>"));
        assert!(html.contains("<span class=\"sum\"></span>"));
    }

    #[test]
    fn every_level_below_the_first_adds_one_guide() {
        let nodes = [branch("a", true, vec![branch("b", true, vec![leaf("c")])])];
        let html = tree(&nodes);
        assert_eq!(html.matches("<span class=\"g\"></span>").count(), 3);
        let c = &html[html.find("<span class=\"id\">c").unwrap()..];
        assert!(c.starts_with("<span class=\"id\">c</span>"));
        let row_c = &html[..html.find("<span class=\"id\">c").unwrap()];
        assert!(row_c.ends_with(
            "<span class=\"g\"></span><span class=\"g\"></span><span class=\"chev\"></span>"
        ));
    }

    #[test]
    fn a_branch_has_an_svg_chevron_and_a_leaf_an_empty_spacer() {
        let html = tree(&[branch("a", true, vec![leaf("b")])]);
        assert!(html.contains("<svg class=\"chev\" viewBox=\"0 0 8 8\" aria-hidden=\"true\"><path d=\"M2.5 1l3.5 3-3.5 3\"/></svg>"));
        assert!(html.contains("<span class=\"chev\"></span>"));
        assert!(!html.contains("data:"));
    }

    #[test]
    fn collapsed_branch_keeps_its_children_and_says_so() {
        let nodes = [branch("Old", false, vec![leaf("hidden")])];
        let html = tree(&nodes);
        assert!(html.contains("aria-expanded=\"false\""));
        assert!(html.contains("hidden"));
        assert_golden("tree_collapsed", &html);
    }

    #[test]
    fn levels_count_from_one_and_grow_with_depth() {
        let nodes = [branch("a", true, vec![branch("b", true, vec![leaf("c")])])];
        let html = tree(&nodes);
        for level in 1..=3 {
            assert!(html.contains(&format!("aria-level=\"{level}\"")), "{level}");
        }
        assert!(!html.contains("aria-level=\"4\""));
    }

    #[test]
    fn a_branch_without_children_has_no_group() {
        let nodes = [branch("a", true, vec![])];
        assert!(!tree(&nodes).contains("role=\"group\""));
    }

    #[test]
    fn the_button_holds_guides_chevron_and_label_and_the_group_follows_the_cells() {
        let html = tree(&[branch("a", true, vec![leaf("b")])]);
        assert!(html.contains("<button type=\"button\"><svg class=\"chev\""));
        assert!(html.contains("<span class=\"id\">a</span></button><span class=\"n\"></span>"));
        assert!(html.contains("</span><ul role=\"group\">"));
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
            cells: Cells {
                count: Some(1),
                summary: Some(hostile_text()),
            },
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
            cells: Cells {
                count: Some(1),
                summary: Some(&long),
            },
            expanded: true,
            children: vec![leaf(&long)],
        }];
        assert_golden("tree_very_long", &tree(&nodes));
    }
}
