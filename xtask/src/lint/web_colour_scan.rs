use super::web_colour::is_ident_char;

fn starts_url(out: &str, rest: &str) -> bool {
    rest.get(..4)
        .is_some_and(|head| head.eq_ignore_ascii_case("url("))
        && out.chars().next_back().is_none_or(|c| !is_ident_char(c))
}

const REGEX_KEYWORDS: &[&str] = &[
    "return",
    "typeof",
    "case",
    "yield",
    "await",
    "void",
    "in",
    "of",
    "delete",
    "new",
    "throw",
    "instanceof",
    "else",
];

fn regex_may_start(out: &str) -> bool {
    let line = out[out.rfind('\n').map_or(0, |at| at + 1)..].trim_end();
    let Some(last) = line.chars().next_back() else {
        return true;
    };
    if "(,=:[!&|?{};>+-*%<~^".contains(last) {
        return true;
    }
    let word_start = line
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_ascii_alphanumeric() || *c == '_' || *c == '$')
        .last()
        .map_or(line.len(), |(at, _)| at);
    let before = line[..word_start].chars().next_back();
    before != Some('.') && REGEX_KEYWORDS.contains(&&line[word_start..])
}

fn regex_len(rest: &str) -> usize {
    let mut in_class = false;
    let mut escaped = false;
    for (at, c) in rest.char_indices().skip(1) {
        if c == '\n' {
            return at;
        }
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '[' => in_class = true,
            ']' => in_class = false,
            '/' if !in_class => return at + 1,
            _ => {}
        }
    }
    rest.len()
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
        } else if line_comments && c == '/' && regex_may_start(&out) {
            let end = regex_len(rest);
            out.push_str(&rest[..end]);
            at += end;
        } else if starts_url(&out, rest) {
            match url_end(&rest[4..]) {
                Some(len) => at += 4 + len,
                None => {
                    out.push_str(&rest[..4]);
                    at += 4;
                }
            }
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

fn url_end(rest: &str) -> Option<usize> {
    let line = &rest[..rest.find('\n').unwrap_or(rest.len())];
    let body = line.trim_start();
    let skipped = line.len() - body.len();
    let search_from = match body.chars().next().filter(|c| *c == '"' || *c == '\'') {
        Some(quote) => skipped + body[1..].find(quote)? + 2,
        None => 0,
    };
    line[search_from..].find(')').map(|at| search_from + at + 1)
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
    fn the_url_call_is_dropped() {
        assert_eq!(strip_comments("a url(x.svg) b", false), "a  b");
        assert_eq!(strip_comments("a url(\"x)y.png\") b", false), "a  b");
        assert_eq!(strip_comments("URL(#add)", false), "");
        assert_eq!(strip_comments("curl(red) b", false), "curl(red) b");
        assert_eq!(strip_comments("none", false), "none");
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
    fn the_contents_of_url_are_dropped_whole() {
        assert_eq!(strip_comments("url(http://x/*y)", true), "");
        assert_eq!(strip_comments("url(\"//x\") // z", true), "  ");
    }

    #[test]
    fn a_url_after_unicode_whitespace_does_not_panic() {
        assert_eq!(strip_comments("url(\u{3000}x.svg) b", false), " b");
        assert_eq!(strip_comments("url(\u{3000}\"x.svg\") b", false), " b");
        assert_eq!(strip_comments("url(\u{3000}\"x", false), "url(\u{3000}\"x");
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

    #[test]
    fn a_regex_literal_is_kept_whole() {
        for text in [
            "x = /\\/\\//; y",
            "x = /[/*]/g; y",
            "f(/a\\/b/, 1); y",
            "  /[//]/.test(s); y",
            "a ? /x/ : /y/; y",
        ] {
            assert_eq!(strip_comments(text, true), text);
        }
    }

    #[test]
    fn a_slash_after_a_value_is_division_and_a_comment_still_strips() {
        assert_eq!(strip_comments("x = a / b; // c", true), "x = a / b;  ");
        assert_eq!(
            strip_comments("x = f(a) / 2; // c", true),
            "x = f(a) / 2;  "
        );
        assert_eq!(strip_comments("/\\//; // c", false), "/\\//; // c");
    }

    #[test]
    fn a_regex_literal_is_only_special_in_scripts() {
        assert_eq!(strip_comments("x = /*c*/ 1", true), "x =   1");
        assert_eq!(strip_comments("x = /a/*c*/", false), "x = /a ");
    }

    #[test]
    fn a_regex_literal_after_an_operator_or_keyword_is_kept_whole() {
        for text in [
            "f = s => /\\/\\//.test(s); y",
            "return /\\/\\//.test(s); y",
            "x = a < /\\/\\//; y",
            "x = typeof /\\/\\//; y",
            "case /\\/\\//: y",
            "  else /\\/\\//.test(s); y",
            "x = 1 - /\\/\\//.x; y",
        ] {
            assert_eq!(strip_comments(text, true), text, "{text}");
        }
    }

    #[test]
    fn a_slash_after_a_name_ending_in_a_keyword_is_division() {
        assert_eq!(
            strip_comments("x = myreturn / 2; // c", true),
            "x = myreturn / 2;  "
        );
        assert_eq!(
            strip_comments("x = a.of / 2; // c", true),
            "x = a.of / 2;  "
        );
    }
}
