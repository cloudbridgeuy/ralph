use crate::config::MermaidSection;
use std::collections::HashSet;
use unicode_width::UnicodeWidthStr;

const SMALL_SOURCE_BYTES: usize = 16 * 1024;
const LARGE_SOURCE_BYTES: usize = 64 * 1024;
const MAX_PARTICIPANTS: usize = 20;
const MAX_STATEMENTS: usize = 2000;
const SEQUENCE_ARROWS: [&str; 8] = ["-->>", "->>", "--x", "-x", "--)", "-)", "-->", "->"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagramKind {
    Flowchart,
    Sequence,
    Er,
    Class,
    State,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Empty,
    Unsupported,
    TooLarge,
    TooManyEdges,
    TooManyRelations,
    TooManyParticipants,
    TooManyStatements,
}

fn body_lines(source: &str) -> impl Iterator<Item = &str> {
    source
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("%%"))
}

fn kind_of(header: &str) -> Option<DiagramKind> {
    let keyword = header
        .split(|c: char| c.is_whitespace() || c == ';')
        .next()
        .unwrap_or("");
    match keyword {
        "flowchart" | "graph" => Some(DiagramKind::Flowchart),
        "sequenceDiagram" => Some(DiagramKind::Sequence),
        "erDiagram" => Some(DiagramKind::Er),
        "classDiagram" => Some(DiagramKind::Class),
        "stateDiagram" | "stateDiagram-v2" => Some(DiagramKind::State),
        _ => None,
    }
}

fn count_link_tokens(source: &str, starts: &[&str]) -> usize {
    let mut count = 0;
    for line in body_lines(source) {
        let mut rest = line;
        while let Some(pos) = starts.iter().filter_map(|s| rest.find(s)).min() {
            count += 1;
            rest = rest[pos..].trim_start_matches(|c| "-=.~>|<*".contains(c));
        }
    }
    count
}

fn sequence_participants(source: &str) -> usize {
    let mut names = HashSet::new();
    for line in body_lines(source).skip(1) {
        if let Some(rest) = line
            .strip_prefix("participant ")
            .or_else(|| line.strip_prefix("actor "))
        {
            names.insert(rest.split(" as ").next().unwrap_or(rest).trim().to_string());
            continue;
        }
        let arrow = SEQUENCE_ARROWS
            .iter()
            .filter_map(|a| line.find(a).map(|p| (p, a.len())))
            .min_by_key(|(p, _)| *p);
        if let Some((pos, len)) = arrow {
            let right = line[pos + len..].split(':').next().unwrap_or("");
            for side in [&line[..pos], right] {
                let name = side.trim().trim_start_matches(['+', '-']);
                if !name.is_empty() {
                    names.insert(name.to_string());
                }
            }
        }
    }
    names.len()
}

fn statement_count(source: &str) -> usize {
    body_lines(source).count().saturating_sub(1)
}

pub fn mermaid_precheck(source: &str, limits: &MermaidSection) -> Result<DiagramKind, Refusal> {
    if source.len() > limits.max_input_bytes {
        return Err(Refusal::TooLarge);
    }
    let header = body_lines(source).next().ok_or(Refusal::Empty)?;
    let kind = kind_of(header).ok_or(Refusal::Unsupported)?;
    match kind {
        DiagramKind::Flowchart | DiagramKind::State => {
            if source.len() > SMALL_SOURCE_BYTES {
                return Err(Refusal::TooLarge);
            }
            let edges = count_link_tokens(source, &["--", "==", "-.", "~~"]);
            if edges > limits.max_flowchart_edges {
                return Err(Refusal::TooManyEdges);
            }
        }
        DiagramKind::Class => {
            let relations = count_link_tokens(source, &["--", ".."]);
            if relations > limits.max_class_relations {
                return Err(Refusal::TooManyRelations);
            }
        }
        DiagramKind::Sequence | DiagramKind::Er => {
            if source.len() > LARGE_SOURCE_BYTES {
                return Err(Refusal::TooLarge);
            }
            if statement_count(source) > MAX_STATEMENTS {
                return Err(Refusal::TooManyStatements);
            }
            if kind == DiagramKind::Sequence && sequence_participants(source) > MAX_PARTICIPANTS {
                return Err(Refusal::TooManyParticipants);
            }
        }
    }
    Ok(kind)
}

pub fn fits_width(output: &str, width: usize) -> bool {
    output
        .lines()
        .map(UnicodeWidthStr::width)
        .max()
        .unwrap_or(0)
        <= width
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> MermaidSection {
        MermaidSection::default()
    }

    fn chain(header: &str, arrow: &str, n: usize) -> String {
        let edges: String = (0..n)
            .map(|i| format!("  N{i} {arrow} N{}\n", i + 1))
            .collect();
        format!("{header}\n{edges}")
    }

    #[test]
    fn corpus_passes() {
        let l = limits();
        let cases = [
            ("graph TD\n  A-->B\n  B-->C", DiagramKind::Flowchart),
            (
                "flowchart LR\n  A -->|yes| B\n  B -.-> C\n  C ==> D",
                DiagramKind::Flowchart,
            ),
            (
                "sequenceDiagram\n  Alice->>Bob: hi\n  Bob-->>Alice: yo",
                DiagramKind::Sequence,
            ),
            (
                "erDiagram\n  CUSTOMER ||--o{ ORDER : places",
                DiagramKind::Er,
            ),
            (
                "classDiagram\n  Animal <|-- Duck\n  Animal <|-- Fish",
                DiagramKind::Class,
            ),
            (
                "stateDiagram\n  [*] --> Idle\n  Idle --> [*]",
                DiagramKind::State,
            ),
            (
                "stateDiagram-v2\n  [*] --> Idle\n  Idle --> [*]",
                DiagramKind::State,
            ),
        ];
        for (src, kind) in cases {
            assert_eq!(mermaid_precheck(src, &l), Ok(kind), "{src}");
        }
    }

    #[test]
    fn unsupported_and_empty_refused() {
        let l = limits();
        assert_eq!(
            mermaid_precheck("gantt\n  title T", &l),
            Err(Refusal::Unsupported)
        );
        assert_eq!(mermaid_precheck("%% only\n\n", &l), Err(Refusal::Empty));
    }

    #[test]
    fn flowchart_edge_limit_counts_tokens_not_lines() {
        let l = limits();
        assert!(mermaid_precheck(&chain("graph TD", "-->", 50), &l).is_ok());
        assert_eq!(
            mermaid_precheck(&chain("graph TD", "-->", 51), &l),
            Err(Refusal::TooManyEdges)
        );
        let one_line = format!("graph LR\n{}", "A-->B-->C-->D;".repeat(17));
        assert_eq!(mermaid_precheck(&one_line, &l), Err(Refusal::TooManyEdges));
    }

    #[test]
    fn state_diagram_shares_flowchart_limits() {
        let l = limits();
        assert!(mermaid_precheck(&chain("stateDiagram-v2", "-->", 50), &l).is_ok());
        assert_eq!(
            mermaid_precheck(&chain("stateDiagram-v2", "-->", 51), &l),
            Err(Refusal::TooManyEdges)
        );
        assert_eq!(
            mermaid_precheck(&chain("stateDiagram", "-->", 51), &l),
            Err(Refusal::TooManyEdges)
        );
        let prefix = "stateDiagram-v2\n%%";
        let suffix = "\n[*] --> A";
        let at = SMALL_SOURCE_BYTES - prefix.len() - suffix.len();
        let ok = format!("{prefix}{}{suffix}", "x".repeat(at));
        assert_eq!(ok.len(), SMALL_SOURCE_BYTES);
        assert!(mermaid_precheck(&ok, &l).is_ok());
        let over = format!("{prefix}{}{suffix}", "x".repeat(at + 1));
        assert_eq!(mermaid_precheck(&over, &l), Err(Refusal::TooLarge));
    }

    #[test]
    fn class_relation_limit() {
        let l = limits();
        assert!(mermaid_precheck(&chain("classDiagram", "<|--", 400), &l).is_ok());
        assert_eq!(
            mermaid_precheck(&chain("classDiagram", "<|--", 401), &l),
            Err(Refusal::TooManyRelations)
        );
    }

    #[test]
    fn input_and_source_size_limits() {
        let small = MermaidSection {
            max_input_bytes: 20,
            ..limits()
        };
        assert!(mermaid_precheck("graph TD\n A-->B", &small).is_ok());
        assert_eq!(
            mermaid_precheck("graph TD\n A-->B-->C-->D", &small),
            Err(Refusal::TooLarge)
        );
        let big_flow = format!("graph TD\n%%{}\nA-->B", "x".repeat(SMALL_SOURCE_BYTES));
        assert_eq!(
            mermaid_precheck(&big_flow, &limits()),
            Err(Refusal::TooLarge)
        );
        let pad = |n: usize| format!("erDiagram\n%%{}\nA ||--o{{ B : r", "x".repeat(n));
        let at = LARGE_SOURCE_BYTES - "erDiagram\n%%\nA ||--o{ B : r".len();
        assert!(mermaid_precheck(&pad(at), &limits()).is_ok());
        assert_eq!(
            mermaid_precheck(&pad(at + 1), &limits()),
            Err(Refusal::TooLarge)
        );
    }

    #[test]
    fn statement_limit() {
        let l = limits();
        let er = |n: usize| format!("erDiagram\n{}", "A ||--o{ B : r\n".repeat(n));
        assert!(mermaid_precheck(&er(2000), &l).is_ok());
        assert_eq!(
            mermaid_precheck(&er(2001), &l),
            Err(Refusal::TooManyStatements)
        );
        let seq = |n: usize| format!("sequenceDiagram\n{}", "A->>B: m\n".repeat(n));
        assert!(mermaid_precheck(&seq(2000), &l).is_ok());
        assert_eq!(
            mermaid_precheck(&seq(2001), &l),
            Err(Refusal::TooManyStatements)
        );
    }

    #[test]
    fn participant_limit() {
        let l = limits();
        let seq = |n: usize| {
            let decl: String = (0..n).map(|i| format!("participant P{i}\n")).collect();
            format!("sequenceDiagram\n{decl}P0->>P1: hi\n")
        };
        assert!(mermaid_precheck(&seq(20), &l).is_ok());
        assert_eq!(
            mermaid_precheck(&seq(21), &l),
            Err(Refusal::TooManyParticipants)
        );
        let implicit: String = (0..21).map(|i| format!("A->>P{i}: x\n")).collect();
        assert_eq!(
            mermaid_precheck(&format!("sequenceDiagram\n{implicit}"), &l),
            Err(Refusal::TooManyParticipants)
        );
    }

    #[test]
    fn width_post_check_uses_display_width() {
        assert!(fits_width("abcd\nab", 4));
        assert!(!fits_width("abcde", 4));
        assert!(fits_width("\u{2500}\u{2500}", 2));
        assert!(!fits_width("\u{4e2d}\u{4e2d}", 3));
    }
}
