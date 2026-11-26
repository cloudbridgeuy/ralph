use super::lines::Statement;
use super::Refusal;

const MAX_FLOWS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SankeyLine {
    Flow,
    Accessibility,
    Unknown,
}

impl SankeyLine {
    fn parse(statement: Statement<'_>) -> Self {
        if statement.code.contains("\"\"") {
            Self::Unknown
        } else if statement.is_accessibility() {
            Self::Accessibility
        } else if statement.code.contains(',') {
            Self::Flow
        } else {
            Self::Unknown
        }
    }
}

struct SankeySize {
    flows: usize,
}

fn size_of(lines: &[SankeyLine]) -> SankeySize {
    SankeySize {
        flows: lines.iter().filter(|&&l| l == SankeyLine::Flow).count(),
    }
}

pub(super) fn check(body: &[Statement<'_>]) -> Result<(), Refusal> {
    let lines: Vec<SankeyLine> = body.iter().copied().map(SankeyLine::parse).collect();
    if lines.contains(&SankeyLine::Unknown) {
        return Err(Refusal::Unsupported);
    }
    if size_of(&lines).flows > MAX_FLOWS {
        return Err(Refusal::TooManyFlows);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::testing::{assert_draws, assert_unsupported, repeated, verdict};
    use super::super::{DiagramKind, Refusal};

    #[test]
    fn sankey_draws_under_both_headers() {
        for header in ["sankey", "sankey-beta"] {
            let source = format!("{header}\n  accTitle: x\n\n  A,B,10\n  \"B, b\",C,5");
            assert_draws(&source, DiagramKind::Sankey);
        }
    }

    #[test]
    fn sankey_refuses_escaped_quotes_and_unknown_lines() {
        assert_unsupported(&["sankey\n  \"a, \"\"x\"\"\",B,10", "sankey\n  A to B"]);
    }

    #[test]
    fn sankey_flow_limit() {
        let sankey = |n| repeated("sankey", n, |i| format!("s{i},t{i},1"));
        assert_draws(&sankey(100), DiagramKind::Sankey);
        assert_eq!(verdict(&sankey(101)), Err(Refusal::TooManyFlows));
    }
}
