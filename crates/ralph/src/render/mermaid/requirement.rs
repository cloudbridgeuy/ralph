use super::lines::Statement;
use super::Refusal;

const MAX_BLOCKS: usize = 30;
const MAX_RELATIONS: usize = 60;
const REQUIREMENT_KEYWORDS: [&str; 6] = [
    "requirement",
    "functionalrequirement",
    "interfacerequirement",
    "performancerequirement",
    "physicalrequirement",
    "designconstraint",
];
const RELATION_KINDS: [&str; 7] = [
    "contains",
    "copies",
    "derives",
    "satisfies",
    "verifies",
    "refines",
    "traces",
];
const RISKS: [&str; 3] = ["low", "medium", "high"];
const VERIFY_METHODS: [&str; 4] = ["analysis", "inspection", "test", "demonstration"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Block {
    Requirement,
    Element,
}

impl Block {
    fn parse_opening(code: &str) -> Option<Self> {
        let body = code.strip_suffix('{')?.trim();
        let (keyword, name) = body.split_once(char::is_whitespace)?;
        if name.trim().is_empty() {
            return None;
        }
        let keyword = keyword.to_lowercase();
        if keyword == "element" {
            return Some(Self::Element);
        }
        REQUIREMENT_KEYWORDS
            .contains(&keyword.as_str())
            .then_some(Self::Requirement)
    }

    fn knows_attribute(self, code: &str) -> bool {
        let Some((key, value)) = code.split_once(':') else {
            return false;
        };
        let value = value.trim().to_lowercase();
        match (self, key) {
            (Self::Requirement, "id" | "text") => true,
            (Self::Requirement, "risk") => RISKS.contains(&value.as_str()),
            (Self::Requirement, "verifymethod") => VERIFY_METHODS.contains(&value.as_str()),
            (Self::Element, "type" | "docref") => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    Top,
    Inside(Block),
}

impl Scope {
    fn after(self, line: ReqLine) -> Self {
        match line {
            ReqLine::Open(block) => Self::Inside(block),
            ReqLine::Close => Self::Top,
            ReqLine::Attribute | ReqLine::Relation | ReqLine::Accessibility | ReqLine::Unknown => {
                self
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReqLine {
    Open(Block),
    Close,
    Attribute,
    Relation,
    Accessibility,
    Unknown,
}

impl ReqLine {
    fn parse(statement: Statement<'_>, scope: Scope) -> Self {
        let code = statement.code;
        if statement.is_accessibility() {
            return Self::Accessibility;
        }
        match scope {
            Scope::Inside(_) if code == "}" => Self::Close,
            Scope::Inside(block) if block.knows_attribute(code) => Self::Attribute,
            Scope::Inside(_) => Self::Unknown,
            Scope::Top => match Block::parse_opening(code) {
                Some(block) => Self::Open(block),
                None if is_relation(code) => Self::Relation,
                None => Self::Unknown,
            },
        }
    }
}

fn is_relation(code: &str) -> bool {
    let Some((source, rest)) = code.split_once(" - ") else {
        return false;
    };
    rest.split_once(" -> ").is_some_and(|(kind, target)| {
        !source.trim().is_empty()
            && !target.trim().is_empty()
            && RELATION_KINDS.contains(&kind.trim().to_lowercase().as_str())
    })
}

fn parse_body(body: &[Statement<'_>]) -> Vec<ReqLine> {
    body.iter()
        .scan(Scope::Top, |scope, &statement| {
            let line = ReqLine::parse(statement, *scope);
            *scope = scope.after(line);
            Some(line)
        })
        .collect()
}

struct ReqSize {
    blocks: usize,
    relations: usize,
    unclosed: bool,
}

fn size_of(lines: &[ReqLine]) -> ReqSize {
    let count = |wanted: fn(&ReqLine) -> bool| lines.iter().filter(|line| wanted(line)).count();
    let blocks = count(|line| matches!(line, ReqLine::Open(_)));
    ReqSize {
        blocks,
        relations: count(|line| *line == ReqLine::Relation),
        unclosed: blocks > count(|line| *line == ReqLine::Close),
    }
}

pub(super) fn check(body: &[Statement<'_>]) -> Result<(), Refusal> {
    let lines = parse_body(body);
    let size = size_of(&lines);
    if lines.contains(&ReqLine::Unknown) || size.unclosed {
        return Err(Refusal::Unsupported);
    }
    if size.blocks > MAX_BLOCKS {
        return Err(Refusal::TooManyBlocks);
    }
    if size.relations > MAX_RELATIONS {
        return Err(Refusal::TooManyRelations);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::testing::{assert_draws, assert_unsupported, repeated, verdict};
    use super::super::{DiagramKind, Refusal};

    #[test]
    fn requirement_draws() {
        assert_draws(
            "requirementDiagram\n  accTitle: x\n  functionalRequirement a {\n    id: 1\n    text: t\n    risk: High\n    verifymethod: Test\n  }\n  element b {\n    type: sim\n    docref: r\n  }\n  b - satisfies -> a",
            DiagramKind::Requirement,
        );
    }

    #[test]
    fn requirement_refuses_blocks_the_renderer_drops_or_trims() {
        assert_unsupported(&[
            "requirementDiagram\n  element b {\n    type: sim",
            "requirementDiagram\n  requirement a {\n    verifyMethod: test\n  }",
            "requirementDiagram\n  requirement a {\n    risk: severe\n  }",
            "requirementDiagram\n  requirement a {\n    type: sim\n  }",
            "requirementDiagram\n  element b {\n    docRef: r\n  }",
        ]);
    }

    #[test]
    fn requirement_refuses_unknown_top_level_lines() {
        assert_unsupported(&[
            "requirementDiagram\n  junk",
            "requirementDiagram\n  type: sim",
            "requirementDiagram\n  }",
            "requirementDiagram\n  widget b {\n  }",
            "requirementDiagram\n  a - owns -> b",
        ]);
    }

    #[test]
    fn requirement_block_limit() {
        let diagram = |n| {
            repeated("requirementDiagram", n, |i| {
                format!("element e{i} {{\n type: t\n }}")
            })
        };
        assert_draws(&diagram(30), DiagramKind::Requirement);
        assert_eq!(verdict(&diagram(31)), Err(Refusal::TooManyBlocks));
    }

    #[test]
    fn requirement_relation_limit() {
        let diagram = |n| repeated("requirementDiagram", n, |i| format!("a{i} - traces -> b"));
        assert_draws(&diagram(60), DiagramKind::Requirement);
        assert_eq!(verdict(&diagram(61)), Err(Refusal::TooManyRelations));
    }
}
