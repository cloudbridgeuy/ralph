use super::Refusal;

const MAX_LINE_BYTES: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Indent(usize);

impl Indent {
    fn of(line: &str) -> Self {
        let columns = line.chars().map_while(|c| match c {
            ' ' => Some(1),
            '\t' => Some(4),
            _ => None,
        });
        Self(columns.sum())
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Statement<'a> {
    pub(super) indent: Indent,
    pub(super) code: &'a str,
}

impl Statement<'_> {
    pub(super) fn has_keyword(self, keyword: &str) -> bool {
        self.code
            .split_at_checked(keyword.len())
            .is_some_and(|(head, rest)| {
                head.eq_ignore_ascii_case(keyword)
                    && rest.starts_with(|c: char| c.is_ascii_whitespace())
            })
    }

    pub(super) fn is_accessibility(self) -> bool {
        self.code.starts_with("accTitle") || self.code.starts_with("accDescr")
    }
}

pub(super) fn statements(source: &str) -> Result<Vec<Statement<'_>>, Refusal> {
    let mut statements = source
        .lines()
        .map(|line| Statement {
            indent: Indent::of(line),
            code: strip_inline_comment(line).trim(),
        })
        .filter(|statement| !statement.code.is_empty())
        .map(|statement| {
            if statement.code.len() > MAX_LINE_BYTES {
                return Err(Refusal::LineTooLong);
            }
            Ok(statement)
        });
    statements.next().transpose()?;
    statements.collect()
}

pub(super) fn strip_inline_comment(line: &str) -> &str {
    let mut quoted = false;
    line.char_indices()
        .find(|&(at, c)| {
            quoted ^= c == '"';
            !quoted && line[at..].starts_with("%%")
        })
        .map_or(line, |(at, _)| &line[..at])
}

pub(super) fn body_lines(source: &str) -> impl Iterator<Item = &str> {
    source
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("%%"))
}

#[cfg(test)]
mod tests {
    use super::super::testing::{assert_draws, verdict};
    use super::super::{DiagramKind, Refusal, SMALL_SOURCE_BYTES};

    #[test]
    fn typed_diagrams_cap_each_line_after_comments() {
        let pie = |n: usize| format!("pie\n  \"{}\" : 1 %% {}", "x".repeat(n), "y".repeat(600));
        let at = 512 - "\"\" : 1".len();
        assert_draws(&pie(at), DiagramKind::Pie);
        assert_eq!(verdict(&pie(at + 1)), Err(Refusal::LineTooLong));
        let header = format!("pie title {}\n  \"a\" : 1", "x".repeat(512));
        assert_eq!(verdict(&header), Err(Refusal::LineTooLong));
    }

    #[test]
    fn typed_diagrams_cap_the_source() {
        let pie = |n: usize| format!("pie\n%%{}\n\"a\" : 1", "x".repeat(n));
        let at = SMALL_SOURCE_BYTES - pie(0).len();
        assert_draws(&pie(at), DiagramKind::Pie);
        assert_eq!(verdict(&pie(at + 1)), Err(Refusal::TooLarge));
    }
}
