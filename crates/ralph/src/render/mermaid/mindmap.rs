use super::lines::{Indent, Statement};
use super::Refusal;

const MAX_NODES: usize = 200;
const MAX_DEPTH: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MindLine {
    Node(Indent),
    Icon,
    Accessibility,
    Unknown,
}

impl MindLine {
    fn parse(statement: Statement<'_>) -> Self {
        let code = statement.code;
        if statement.is_accessibility() {
            Self::Accessibility
        } else if code.starts_with("::icon(") {
            Self::Icon
        } else if code.contains(":::") || code.contains("::icon(") {
            Self::Unknown
        } else {
            Self::Node(statement.indent)
        }
    }
}

struct MindSize {
    nodes: usize,
    roots: usize,
    depth: usize,
}

fn size_of(lines: &[MindLine]) -> MindSize {
    let mut size = MindSize {
        nodes: 0,
        roots: 0,
        depth: 0,
    };
    let mut ancestors: Vec<Indent> = Vec::new();
    for line in lines {
        let MindLine::Node(indent) = *line else {
            continue;
        };
        while ancestors.last().is_some_and(|&last| last >= indent) {
            ancestors.pop();
        }
        size.nodes += 1;
        size.roots += usize::from(ancestors.is_empty());
        ancestors.push(indent);
        size.depth = size.depth.max(ancestors.len());
    }
    size
}

pub(super) fn check(body: &[Statement<'_>]) -> Result<(), Refusal> {
    let lines: Vec<MindLine> = body.iter().copied().map(MindLine::parse).collect();
    let size = size_of(&lines);
    if lines.contains(&MindLine::Unknown) || size.roots > 1 {
        return Err(Refusal::Unsupported);
    }
    if size.nodes > MAX_NODES {
        return Err(Refusal::TooManyNodes);
    }
    if size.depth > MAX_DEPTH {
        return Err(Refusal::TooDeep);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::testing::{assert_draws, assert_unsupported, repeated, verdict};
    use super::super::{DiagramKind, Refusal};

    #[test]
    fn mindmap_draws() {
        assert_draws(
            "mindmap\n  accTitle: x\n  root((Root))\n    A\n    ::icon(fa fa-book)\n      B[Bee]\n    C",
            DiagramKind::Mindmap,
        );
    }

    #[test]
    fn mindmap_refuses_a_second_root_and_inline_decorations() {
        assert_unsupported(&[
            "mindmap\n  Root\n    A\n  Other",
            "mindmap\n\tRoot\n   A",
            "mindmap\n  Root\n    A:::urgent",
            "mindmap\n  Root\n    A ::icon(fa fa-book)",
        ]);
    }

    #[test]
    fn mindmap_node_limit() {
        let mindmap = |n| repeated("mindmap\n root", n, |i| format!("n{i}"));
        assert_draws(&mindmap(199), DiagramKind::Mindmap);
        assert_eq!(verdict(&mindmap(200)), Err(Refusal::TooManyNodes));
    }

    #[test]
    fn mindmap_depth_limit_counts_nesting_not_columns() {
        let mindmap = |n: usize| repeated("mindmap", n, |i| format!("{}n{i}", "   ".repeat(i)));
        assert_draws(&mindmap(12), DiagramKind::Mindmap);
        assert_eq!(verdict(&mindmap(13)), Err(Refusal::TooDeep));
    }
}
