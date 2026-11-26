use super::lines::Statement;
use super::Refusal;

const MAX_POINTS: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuadLine {
    Title,
    Axis,
    Label,
    Point,
    Accessibility,
    Unknown,
}

impl QuadLine {
    fn parse(statement: Statement<'_>) -> Self {
        let code = statement.code;
        let starts = |prefixes: &[&str]| prefixes.iter().any(|p| code.starts_with(p));
        if statement.is_accessibility() {
            Self::Accessibility
        } else if starts(&["title "]) {
            Self::Title
        } else if starts(&["x-axis ", "y-axis "]) {
            Self::Axis
        } else if starts(&["quadrant-1 ", "quadrant-2 ", "quadrant-3 ", "quadrant-4 "]) {
            Self::Label
        } else if is_point(code) {
            Self::Point
        } else {
            Self::Unknown
        }
    }
}

fn is_point(code: &str) -> bool {
    !code.contains(":::")
        && code
            .split_once(": [")
            .is_some_and(|(name, _)| !name.trim().is_empty())
}

struct QuadSize {
    points: usize,
}

fn size_of(lines: &[QuadLine]) -> QuadSize {
    QuadSize {
        points: lines.iter().filter(|&&l| l == QuadLine::Point).count(),
    }
}

pub(super) fn check(body: &[Statement<'_>]) -> Result<(), Refusal> {
    let lines: Vec<QuadLine> = body.iter().copied().map(QuadLine::parse).collect();
    if lines.contains(&QuadLine::Unknown) {
        return Err(Refusal::Unsupported);
    }
    if size_of(&lines).points > MAX_POINTS {
        return Err(Refusal::TooManyPoints);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::testing::{assert_draws, assert_unsupported, repeated, verdict};
    use super::super::{DiagramKind, Refusal};

    #[test]
    fn quadrant_draws() {
        assert_draws(
            "quadrantChart\n  title Reach\n  accTitle: x\n  x-axis Low --> High\n  y-axis Few --> Many\n  quadrant-1 Expand\n  quadrant-2 Promote\n  quadrant-3 Drop\n  quadrant-4 Improve\n  Campaign A: [0.3, 0.6]",
            DiagramKind::Quadrant,
        );
    }

    #[test]
    fn quadrant_refuses_lines_the_renderer_drops_or_mangles() {
        assert_unsupported(&[
            "quadrantChart\n  A:[0.3, 0.6]",
            "quadrantChart\n  junk",
            "quadrantChart\n  A:::hot: [0.3, 0.6]",
            "quadrantChart\n  classDef hot color: #f00",
            "quadrantChart\n  : [0.3, 0.6]",
        ]);
    }

    #[test]
    fn quadrant_point_limit() {
        let chart = |n| repeated("quadrantChart", n, |i| format!("p{i}: [0.5, 0.5]"));
        assert_draws(&chart(20), DiagramKind::Quadrant);
        assert_eq!(verdict(&chart(21)), Err(Refusal::TooManyPoints));
    }
}
