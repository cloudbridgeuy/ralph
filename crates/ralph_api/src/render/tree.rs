use crate::render::{empty, escape};

/// A half-open range of entries `lo..hi` inside a thread of `total` entries.
pub struct Extent {
    pub lo: usize,
    pub hi: usize,
    pub total: usize,
}

#[derive(Default)]
pub struct Cells<'a> {
    pub count: Option<usize>,
    pub extent: Option<Extent>,
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

/// The bar is 100 units wide whatever the cell width, and a span is never thinner than this.
const MIN_SPAN: f64 = 1.0;

/// Returns the start and width of the span, in percent of the track.
fn extent_geometry(extent: &Extent) -> Option<(f64, f64)> {
    if extent.total == 0 || extent.lo >= extent.hi || extent.lo >= extent.total {
        return None;
    }
    let percent = |entries: usize| entries as f64 * 100.0 / extent.total as f64;
    let width = percent(extent.hi.min(extent.total) - extent.lo).max(MIN_SPAN);
    let start = percent(extent.lo).min(100.0 - width);
    Some((start, width))
}

fn number(value: f64) -> String {
    let text = format!("{value:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn extent_html(extent: Option<&Extent>) -> String {
    match extent.and_then(extent_geometry) {
        Some((x, width)) => format!(
            "<svg class=\"extent\" viewBox=\"0 0 100 6\" preserveAspectRatio=\"none\" aria-hidden=\"true\"><rect class=\"track\" y=\"2\" width=\"100\" height=\"2\"/><rect class=\"span\" x=\"{}\" width=\"{}\" height=\"6\"/></svg>",
            number(x),
            number(width)
        ),
        None => "<span class=\"extent\"></span>".to_string(),
    }
}

fn cells_html(cells: &Cells) -> String {
    let count = cells.count.map(|n| n.to_string()).unwrap_or_default();
    let summary = cells.summary.map(escape).unwrap_or_default();
    format!(
        "<span class=\"n\">{count}</span>{}<span class=\"sum\">{summary}</span>",
        extent_html(cells.extent.as_ref())
    )
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
    use super::{extent_geometry, tree, Cells, Extent, TreeNode};
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
                ..Cells::default()
            },
            expanded: true,
            children: vec![TreeNode::Leaf {
                label: "#4 user",
                cells: Cells {
                    count: None,
                    summary: Some("hello"),
                    ..Cells::default()
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
                ..Cells::default()
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
                ..Cells::default()
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
                ..Cells::default()
            },
            expanded: true,
            children: vec![leaf(&long)],
        }];
        assert_golden("tree_very_long", &tree(&nodes));
    }

    fn extent(lo: usize, hi: usize, total: usize) -> Extent {
        Extent { lo, hi, total }
    }

    fn row_with(extent: Option<Extent>) -> String {
        tree(&[TreeNode::Leaf {
            label: "x",
            cells: Cells {
                extent,
                ..Cells::default()
            },
        }])
    }

    #[test]
    fn the_extent_is_a_percent_of_the_thread() {
        assert_eq!(extent_geometry(&extent(0, 50, 100)), Some((0.0, 50.0)));
        assert_eq!(extent_geometry(&extent(25, 75, 100)), Some((25.0, 50.0)));
    }

    #[test]
    fn a_one_entry_node_in_a_huge_thread_keeps_a_visible_span() {
        for lo in [0, 2_500, 4_999] {
            let (x, width) = extent_geometry(&extent(lo, lo + 1, 5_000)).unwrap();
            assert!(width >= 1.0, "{lo}: {width}");
            assert!(x >= 0.0 && x + width <= 100.0, "{lo}: {x} {width}");
        }
        let (_, width) = extent_geometry(&extent(0, 1, 1_000_000)).unwrap();
        assert!(width >= 1.0);
    }

    #[test]
    fn a_span_past_the_total_is_clamped_to_the_track() {
        let (x, width) = extent_geometry(&extent(90, 500, 100)).unwrap();
        assert_eq!((x, width), (90.0, 10.0));
    }

    #[test]
    fn a_degenerate_extent_draws_no_bar() {
        for bad in [
            extent(0, 1, 0),
            extent(5, 5, 10),
            extent(7, 3, 10),
            extent(10, 12, 10),
        ] {
            assert_eq!(extent_geometry(&bad), None);
        }
        assert!(row_with(Some(extent(5, 5, 10))).contains("<span class=\"extent\"></span>"));
    }

    #[test]
    fn no_extent_leaves_an_empty_cell_so_columns_hold() {
        let html = row_with(None);
        assert!(html.contains("<span class=\"extent\"></span>"));
        assert!(!html.contains("<rect"));
    }

    #[test]
    fn the_bar_stretches_with_the_cell_and_uses_no_inline_style() {
        let html = row_with(Some(extent(10, 20, 100)));
        assert!(html
            .contains("viewBox=\"0 0 100 6\" preserveAspectRatio=\"none\" aria-hidden=\"true\""));
        assert!(html.contains("<rect class=\"track\" y=\"2\" width=\"100\" height=\"2\"/>"));
        assert!(html.contains("<rect class=\"span\" x=\"10\" width=\"10\" height=\"6\"/>"));
        assert!(!html.contains("style="));
        assert!(!html.contains("data:"));
    }

    #[test]
    fn extent_numbers_have_at_most_two_decimals() {
        let html = row_with(Some(extent(1, 2, 3)));
        assert!(html.contains("x=\"33.33\" width=\"33.33\""), "{html}");
    }

    #[test]
    fn rows_with_an_extent_bar() {
        let nodes = [TreeNode::Branch {
            label: "0-31",
            cells: Cells {
                count: Some(32),
                summary: Some("Early work"),
                extent: Some(extent(0, 32, 45)),
            },
            expanded: true,
            children: vec![TreeNode::Leaf {
                label: "#40 user",
                cells: Cells::default(),
            }],
        }];
        assert_golden("tree_extent", &tree(&nodes));
    }
}
