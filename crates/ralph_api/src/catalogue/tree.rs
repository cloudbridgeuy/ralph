use super::{group, text};
use crate::render::{error, tree, Cells, Extent, TreeNode};

const HOSTILE: &str = "</r-tree><script>alert(\"x\")</script> 'quote' & \"double\"";

fn spanned(mut node: TreeNode<'_>, lo: usize, hi: usize, total: usize) -> TreeNode<'_> {
    if let TreeNode::Branch { cells, .. } | TreeNode::Leaf { cells, .. } = &mut node {
        cells.extent = Some(Extent { lo, hi, total });
    }
    node
}

fn cells(summary: Option<&str>) -> Cells<'_> {
    Cells {
        summary,
        ..Cells::default()
    }
}

fn branch<'a>(
    label: &'a str,
    count: usize,
    expanded: bool,
    children: Vec<TreeNode<'a>>,
) -> TreeNode<'a> {
    TreeNode::Branch {
        label,
        cells: Cells {
            count: Some(count),
            ..Cells::default()
        },
        expanded,
        children,
    }
}

fn leaf<'a>(label: &'a str, summary: Option<&'a str>) -> TreeNode<'a> {
    TreeNode::Leaf {
        label,
        cells: cells(summary),
    }
}

fn earlier(expanded: bool) -> Vec<TreeNode<'static>> {
    vec![branch(
        "Earlier work",
        48,
        expanded,
        vec![leaf("Ported the nap pass", None)],
    )]
}

fn sparse() -> Vec<TreeNode<'static>> {
    vec![
        branch(
            "Session summary",
            12,
            true,
            vec![leaf("Chose SQLite for the store", Some("decision"))],
        ),
        branch(
            "Earlier work",
            48,
            false,
            vec![leaf("Ported the nap pass", None)],
        ),
        leaf("Open question on token limits", None),
    ]
}

fn dense() -> Vec<TreeNode<'static>> {
    vec![branch(
        "Memory root",
        212,
        true,
        vec![
            spanned(
                branch(
                    "Week 1",
                    90,
                    true,
                    vec![
                        branch(
                            "Monday",
                            30,
                            true,
                            vec![
                                leaf("Set up the workspace", None),
                                leaf("Wrote the clap router", Some("done")),
                                leaf("Fixed the flaky timer test", None),
                            ],
                        ),
                        branch(
                            "Tuesday",
                            60,
                            false,
                            vec![leaf("Reworked the stream collector", None)],
                        ),
                    ],
                ),
                0,
                90,
                212,
            ),
            spanned(
                branch(
                    "Week 2",
                    122,
                    true,
                    vec![
                        branch(
                            "Thursday",
                            40,
                            true,
                            vec![leaf("Added the wire proxy", Some("merged"))],
                        ),
                        leaf("Reviewed the release notes", None),
                    ],
                ),
                90,
                212,
                212,
            ),
            leaf("Parked the thread merge idea", None),
        ],
    )]
}

pub(super) fn trees(very_long_summary: &str) -> String {
    let unbroken = "memory-summary-".repeat(34);
    let failed = vec![
        branch(
            "Session summary",
            12,
            true,
            vec![TreeNode::Row(error("could not load the children"))],
        ),
        leaf("Open question on token limits", None),
    ];
    let long = vec![
        branch(&unbroken, 1, true, vec![leaf(&unbroken, None)]),
        TreeNode::Branch {
            label: very_long_summary,
            cells: Cells {
                count: Some(1200),
                summary: Some(very_long_summary),
                ..Cells::default()
            },
            expanded: true,
            children: vec![],
        },
    ];
    let hostile = vec![TreeNode::Branch {
        label: HOSTILE,
        cells: Cells {
            count: Some(1),
            summary: Some(HOSTILE),
            ..Cells::default()
        },
        expanded: true,
        children: vec![leaf(HOSTILE, None)],
    }];
    group(
        "r-tree",
        &[
            ("empty", tree(&[])),
            ("sparse", tree(&sparse())),
            ("dense", tree(&dense())),
            ("error", tree(&failed)),
            ("very-long", tree(&long)),
            ("collapsed", tree(&earlier(false))),
            ("expanded", tree(&earlier(true))),
            ("hostile", tree(&hostile)),
            (
                "focus",
                format!(
                    "{}{}",
                    text("Press Tab to reach a row. The focused row shows an outline."),
                    tree(&earlier(false))
                ),
            ),
        ],
    )
}

#[cfg(test)]
mod tests {
    use crate::catalogue::page;

    const COMMITTED: &str = include_str!("../../fixtures/catalogue.json");
    const APP_CSS: &str = include_str!("../../assets/app.css");

    fn tree_groups() -> Vec<String> {
        let marker = "<div class=\"group\"><h3>r-tree</h3>";
        page(COMMITTED)
            .split(marker)
            .skip(1)
            .map(|rest| {
                rest.split("<div class=\"group\">")
                    .next()
                    .unwrap_or("")
                    .split("</section>")
                    .next()
                    .unwrap_or("")
                    .to_string()
            })
            .collect()
    }

    fn names(group: &str) -> Vec<&str> {
        group
            .split("<h4>")
            .skip(1)
            .filter_map(|part| part.split("</h4>").next())
            .collect()
    }

    #[test]
    fn every_tree_state_shows_in_both_columns() {
        let groups = tree_groups();
        assert_eq!(groups.len(), 2);
        for group in &groups {
            assert_eq!(
                names(group),
                [
                    "empty",
                    "sparse",
                    "dense",
                    "error",
                    "very-long",
                    "collapsed",
                    "expanded",
                    "hostile",
                    "focus"
                ]
            );
        }
    }

    #[test]
    fn the_empty_state_holds_r_empty_and_the_error_state_r_error() {
        for group in tree_groups() {
            let parts: Vec<&str> = group.split("<div class=\"state\">").collect();
            assert!(parts[1].contains("<r-tree><r-empty>"));
            assert!(parts[4].contains("<r-error"));
        }
    }

    #[test]
    fn the_dense_state_reaches_four_levels() {
        for group in tree_groups() {
            assert!(group.contains("aria-level=\"4\""));
            assert!(!group.contains("aria-level=\"9\""));
        }
    }

    #[test]
    fn the_very_long_state_has_a_500_character_unbroken_label() {
        let page = page(COMMITTED);
        let longest = page
            .split(|c: char| c.is_whitespace() || c == '<' || c == '>')
            .map(str::len)
            .max()
            .unwrap_or(0);
        assert!(longest >= 500);
    }

    #[test]
    fn the_hostile_label_is_escaped() {
        let html = page(COMMITTED);
        assert!(!html.contains("<script"));
        assert!(html.contains("&lt;/r-tree&gt;"));
        assert!(html.contains("&lt;script&gt;"));
        assert_eq!(
            html.matches("</r-tree>").count(),
            html.matches("<r-tree>").count()
        );
    }

    #[test]
    fn the_tree_markup_has_no_inline_style_and_only_native_tags() {
        for group in tree_groups() {
            assert!(!group.contains("style="));
            let tree_part: String = group
                .split("<r-tree>")
                .skip(1)
                .filter_map(|part| part.split("</r-tree>").next())
                .collect::<Vec<_>>()
                .join(" ");
            for tag in tree_part.split('<').skip(1) {
                let name: String = tag
                    .trim_start_matches('/')
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                    .collect();
                let allowed = [
                    "ul", "li", "button", "span", "svg", "path", "rect", "r-empty", "r-error", "p",
                ];
                assert!(allowed.contains(&name.as_str()), "{name}");
            }
        }
    }

    #[test]
    fn app_css_styles_the_tree() {
        for needle in [
            "r-tree ",
            "[aria-expanded=\"false\"]",
            ":focus-visible",
            "overflow-wrap: anywhere",
        ] {
            assert!(APP_CSS.contains(needle), "{needle}");
        }
        assert!(APP_CSS.contains("r-tree button:focus-visible"));
    }
}
