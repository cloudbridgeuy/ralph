use super::web_colour::is_ident_char;

fn starts_url(out: &str, rest: &str) -> bool {
    rest.get(..4)
        .is_some_and(|head| head.eq_ignore_ascii_case("url("))
        && out.chars().next_back().is_none_or(|c| !is_ident_char(c))
}

pub(super) fn strip_comments(text: &str, line_comments: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut quote: Option<char> = None;
    let mut at = 0;
    while let Some(c) = text[at..].chars().next() {
        let rest = &text[at..];
        if let Some(open) = quote {
            out.push(c);
            at += c.len_utf8();
            if c == '\\' {
                if let Some(escaped) = text[at..].chars().next() {
                    out.push(escaped);
                    at += escaped.len_utf8();
                }
            } else if c == open || (c == '\n' && open != '`') {
                quote = None;
            }
        } else if let Some(body) = rest.strip_prefix("/*") {
            let end = body.find("*/").map_or(rest.len(), |found| found + 4);
            out.push(' ');
            out.extend(rest[..end].chars().filter(|c| *c == '\n'));
            at += end;
        } else if line_comments && rest.starts_with("//") {
            out.push(' ');
            at += rest.find('\n').unwrap_or(rest.len());
        } else if starts_url(&out, rest) {
            let end = 4 + url_end(&rest[4..]);
            out.push_str(&rest[..end]);
            at += end;
        } else {
            if matches!(c, '"' | '\'') || (line_comments && c == '`') {
                quote = Some(c);
            }
            out.push(c);
            at += c.len_utf8();
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Terminator {
    Semicolon,
    OpenBrace,
    CloseBrace,
    End,
}

pub(super) struct Statement<'a> {
    pub(super) line: usize,
    pub(super) text: &'a str,
    pub(super) terminator: Terminator,
}

pub(super) fn statement_between(
    text: &str,
    start: usize,
    end: usize,
    terminator: Terminator,
) -> Option<Statement<'_>> {
    let piece = &text[start..end];
    let body = piece.trim_start();
    if body.trim().is_empty() {
        return None;
    }
    let lead = &piece[..piece.len() - body.len()];
    Some(Statement {
        line: 1 + text[..start].matches('\n').count() + lead.matches('\n').count(),
        text: body.trim_end(),
        terminator,
    })
}

pub(super) fn statements(text: &str) -> Vec<Statement<'_>> {
    let mut found = Vec::new();
    let mut quote: Option<char> = None;
    let mut start = 0;
    for (at, c) in text.char_indices() {
        match (quote, c) {
            (Some(_), '\n') => quote = None,
            (Some(open), c) if c == open => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, ';' | '{' | '}') => {
                let terminator = match c {
                    ';' => Terminator::Semicolon,
                    '{' => Terminator::OpenBrace,
                    _ => Terminator::CloseBrace,
                };
                found.extend(statement_between(text, start, at, terminator));
                start = at + 1;
            }
            (None, _) => {}
        }
    }
    found.extend(statement_between(text, start, text.len(), Terminator::End));
    found
}

pub(super) fn url_end(rest: &str) -> usize {
    let line_end = rest.find('\n').unwrap_or(rest.len());
    let body = rest.trim_start();
    let skipped = rest.len() - body.len();
    let search_from = match body.chars().next().filter(|c| *c == '"' || *c == '\'') {
        Some(quote) => match body[1..].find(quote) {
            Some(at) if skipped + at + 1 < line_end => skipped + at + 2,
            _ => return line_end,
        },
        None => 0,
    };
    rest[search_from..]
        .find(')')
        .map_or(rest.len(), |at| search_from + at + 1)
}

pub(super) fn without_urls(line: &str) -> String {
    const NAME: &str = "url(";
    let lower = line.to_ascii_lowercase();
    let mut out = String::new();
    let mut from = 0;
    while let Some(found) = lower[from..].find(NAME) {
        let at = from + found;
        let boundary = line[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !is_ident_char(c));
        if !boundary {
            out.push_str(&line[from..at + NAME.len()]);
            from = at + NAME.len();
            continue;
        }
        out.push_str(&line[from..at]);
        let end = at + NAME.len() + url_end(&line[at + NAME.len()..]);
        out.extend(line[at..end].chars().filter(|c| *c == '\n'));
        from = end;
    }
    out.push_str(&line[from..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn statement_parts(text: &str) -> Vec<(usize, &str, Terminator)> {
        statements(text)
            .into_iter()
            .map(|s| (s.line, s.text, s.terminator))
            .collect()
    }

    #[test]
    fn statements_split_at_semicolons_and_braces() {
        assert_eq!(
            statement_parts("a {\n  color:\n    red;\n  margin: 0\n}\nb"),
            vec![
                (1, "a", Terminator::OpenBrace),
                (2, "color:\n    red", Terminator::Semicolon),
                (4, "margin: 0", Terminator::CloseBrace),
                (6, "b", Terminator::End),
            ]
        );
    }

    #[test]
    fn statements_skip_blank_pieces_and_quoted_separators() {
        assert_eq!(statement_parts(" ; } \n"), vec![]);
        assert_eq!(
            statement_parts("a::after { content: \"{;}\"; }"),
            vec![
                (1, "a::after", Terminator::OpenBrace),
                (1, "content: \"{;}\"", Terminator::Semicolon),
            ]
        );
    }

    #[test]
    fn without_urls_drops_the_url_call() {
        assert_eq!(without_urls("a url(x.svg) b"), "a  b");
        assert_eq!(without_urls("a url(\"x)y.png\") b"), "a  b");
        assert_eq!(without_urls("URL(#add)"), "");
        assert_eq!(without_urls("curl(red) b"), "curl(red) b");
        assert_eq!(without_urls("none"), "none");
    }

    #[test]
    fn block_comments_are_blanked_and_keep_their_newlines() {
        assert_eq!(
            strip_comments("a/* x\ny */b", false),
            "a b".replace(' ', " \n")
        );
        assert_eq!(strip_comments("a /* x */ b\nc", false), "a   b\nc");
        assert_eq!(strip_comments("a /* open\nstill", false), "a  \n");
    }

    #[test]
    fn line_comments_are_stripped_only_when_asked() {
        assert_eq!(strip_comments("a // x\nb", true), "a  \nb");
        assert_eq!(strip_comments("a // x\nb", false), "a // x\nb");
    }

    #[test]
    fn comment_openers_inside_strings_are_text() {
        assert_eq!(
            strip_comments("u = \"http://x\"; v", true),
            "u = \"http://x\"; v"
        );
        assert_eq!(strip_comments("u = '/* y'; v", true), "u = '/* y'; v");
        assert_eq!(strip_comments("u = `//${a}`; v", true), "u = `//${a}`; v");
        assert_eq!(
            strip_comments("u = \"a\\\"//b\"; v", true),
            "u = \"a\\\"//b\"; v"
        );
    }

    #[test]
    fn an_unclosed_string_ends_at_the_line() {
        assert_eq!(strip_comments("don't\n// x", true), "don't\n ");
    }

    #[test]
    fn the_contents_of_url_are_kept_whole() {
        assert_eq!(strip_comments("url(http://x/*y)", true), "url(http://x/*y)");
        assert_eq!(strip_comments("url(\"//x\") // z", true), "url(\"//x\")  ");
    }

    #[test]
    fn a_url_after_unicode_whitespace_does_not_panic() {
        assert_eq!(without_urls("url(\u{3000}x.svg) b"), " b");
        assert_eq!(without_urls("url(\u{3000}\"x.svg\") b"), " b");
        assert_eq!(without_urls("url(\u{3000}\"x"), "");
        assert_eq!(strip_comments("url(\u{3000}\"x", false), "url(\u{3000}\"x");
    }

    #[test]
    fn an_unclosed_quote_ends_the_statement_quote_at_the_newline() {
        assert_eq!(
            statement_parts("a { x: \"q;\nb: c; }"),
            vec![
                (1, "a", Terminator::OpenBrace),
                (1, "x: \"q;\nb: c", Terminator::Semicolon),
            ]
        );
    }
}
