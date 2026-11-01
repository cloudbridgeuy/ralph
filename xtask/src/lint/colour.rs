use crate::prelude::*;

const ALLOWLIST: &[&str] = &["crates/core/src/ansi.rs", "crates/core/src/kitty.rs"];

#[derive(Debug, PartialEq, Eq)]
pub struct ColourViolation {
    pub line: usize,
    pub snippet: String,
}

fn is_exempt_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    if !normalized.starts_with("crates/") || ALLOWLIST.contains(&normalized.as_str()) {
        return true;
    }
    let mut parts = normalized.split('/').peekable();
    while let Some(part) = parts.next() {
        let is_last = parts.peek().is_none();
        if (!is_last && (part == "tests" || part == "vendor"))
            || (is_last && (part == "tests.rs" || part.ends_with("_tests.rs")))
        {
            return true;
        }
    }
    false
}

fn is_colour_param(param: &str) -> bool {
    matches!(
        param.parse::<u32>(),
        Ok(30..=38 | 40..=48 | 90..=97 | 100..=107)
    )
}

fn sgr_sets_colour(after_prefix: &str) -> bool {
    let params: String = after_prefix
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == ';')
        .collect();
    let terminated = after_prefix[params.len()..].starts_with('m');
    let mut parts: Vec<&str> = params.split(';').collect();
    if !terminated {
        parts.pop();
    }
    parts.into_iter().any(is_colour_param)
}

fn escape_prefix_len(escaped: &str) -> Option<usize> {
    let lower = escaped.to_ascii_lowercase();
    if lower.starts_with("x1b[") {
        return Some(4);
    }
    if lower.starts_with("033[") {
        return Some(4);
    }
    let digits = lower.strip_prefix("u{")?;
    let close = digits.find('}')?;
    let value = &digits[..close];
    let is_escape = (1..=6).contains(&value.len())
        && value.trim_start_matches('0') == "1b"
        && digits[close + 1..].starts_with('[');
    is_escape.then_some(2 + close + 2)
}

fn line_has_colour_escape(line: &str) -> bool {
    line.match_indices('\\').any(|(at, _)| {
        let escaped = &line[at + 1..];
        escape_prefix_len(escaped).is_some_and(|len| sgr_sets_colour(&escaped[len..]))
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lex {
    Code,
    Str,
    Raw(usize),
    Block(usize),
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct LineBraces {
    opens: i64,
    delta: i64,
    starts_in_code: bool,
}

fn raw_string_open(chars: &[char], at: usize) -> Option<(usize, usize)> {
    if chars[at] != 'r' {
        return None;
    }
    let after_ident = at > 0 && (chars[at - 1].is_alphanumeric() || chars[at - 1] == '_');
    let byte_prefix = at > 0 && chars[at - 1] == 'b';
    if after_ident && !byte_prefix {
        return None;
    }
    let hashes = chars[at + 1..].iter().take_while(|c| **c == '#').count();
    (chars.get(at + 1 + hashes) == Some(&'"')).then_some((hashes, at + 2 + hashes))
}

fn char_literal_len(chars: &[char], at: usize) -> Option<usize> {
    match chars.get(at + 1)? {
        '\\' => chars[at + 2..]
            .iter()
            .position(|c| *c == '\'')
            .map(|close| close + 3),
        _ => (chars.get(at + 2) == Some(&'\'')).then_some(3),
    }
}

fn raw_string_closes(chars: &[char], at: usize, hashes: usize) -> bool {
    chars[at] == '"'
        && chars.len() >= at + 1 + hashes
        && chars[at + 1..at + 1 + hashes].iter().all(|c| *c == '#')
}

fn scan_line(mut state: Lex, line: &str) -> (Lex, LineBraces) {
    let chars: Vec<char> = line.chars().collect();
    let mut braces = LineBraces {
        starts_in_code: state == Lex::Code,
        ..LineBraces::default()
    };
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        match state {
            Lex::Str => {
                match c {
                    '\\' => at += 1,
                    '"' => state = Lex::Code,
                    _ => {}
                }
                at += 1;
            }
            Lex::Raw(hashes) => {
                if raw_string_closes(&chars, at, hashes) {
                    state = Lex::Code;
                    at += hashes;
                }
                at += 1;
            }
            Lex::Block(depth) => {
                if c == '/' && chars.get(at + 1) == Some(&'*') {
                    state = Lex::Block(depth + 1);
                    at += 2;
                } else if c == '*' && chars.get(at + 1) == Some(&'/') {
                    state = if depth == 1 {
                        Lex::Code
                    } else {
                        Lex::Block(depth - 1)
                    };
                    at += 2;
                } else {
                    at += 1;
                }
            }
            Lex::Code => {
                if c == '/' && chars.get(at + 1) == Some(&'/') {
                    break;
                }
                if c == '/' && chars.get(at + 1) == Some(&'*') {
                    state = Lex::Block(1);
                    at += 2;
                    continue;
                }
                if let Some((hashes, next)) = raw_string_open(&chars, at) {
                    state = Lex::Raw(hashes);
                    at = next;
                    continue;
                }
                if c == '\'' {
                    at += char_literal_len(&chars, at).unwrap_or(1);
                    continue;
                }
                match c {
                    '"' => state = Lex::Str,
                    '{' => {
                        braces.opens += 1;
                        braces.delta += 1;
                    }
                    '}' => braces.delta -= 1,
                    _ => {}
                }
                at += 1;
            }
        }
    }
    (state, braces)
}

fn line_braces(lines: &[&str]) -> Vec<LineBraces> {
    let mut state = Lex::Code;
    lines
        .iter()
        .map(|line| {
            let (next, braces) = scan_line(state, line);
            state = next;
            braces
        })
        .collect()
}

fn test_module_lines(lines: &[&str]) -> Vec<bool> {
    let braces = line_braces(lines);
    let mut skipped = vec![false; lines.len()];
    let mut index = 0;
    while index < lines.len() {
        if lines[index].trim() != "#[cfg(test)]" || !braces[index].starts_in_code {
            index += 1;
            continue;
        }
        let mut depth = 0i64;
        let mut opened = false;
        let mut end = index;
        for (offset, line) in lines[index..].iter().enumerate() {
            end = index + offset;
            depth += braces[end].delta;
            opened |= braces[end].opens > 0;
            if (opened && depth <= 0) || (!opened && offset > 0 && line.trim_end().ends_with(';')) {
                break;
            }
        }
        skipped[index..=end].fill(true);
        index = end + 1;
    }
    skipped
}

pub fn find_raw_colour_escapes(path: &str, text: &str) -> Vec<ColourViolation> {
    if is_exempt_path(path) {
        return Vec::new();
    }
    let lines: Vec<&str> = text.lines().collect();
    let skipped = test_module_lines(&lines);
    lines
        .iter()
        .enumerate()
        .filter(|(index, line)| {
            !skipped[*index] && !line.trim_start().starts_with("//") && line_has_colour_escape(line)
        })
        .map(|(index, line)| ColourViolation {
            line: index + 1,
            snippet: line.trim().to_string(),
        })
        .collect()
}

pub async fn run_raw_colour_check(global: &crate::Global) -> super::error::Result<bool> {
    if !global.is_silent() {
        aprintln!(
            "{} {}",
            p_b("🔧"),
            p_b("Checking for raw colour escapes...")
        );
    }

    let output = tokio::process::Command::new("git")
        .args(["ls-files", "crates/**/*.rs"])
        .output()
        .await?;

    if !output.status.success() {
        aprintln!("{} Failed to list Rust files", p_r("❌"));
        return Ok(false);
    }

    let files = String::from_utf8_lossy(&output.stdout);
    let mut total = 0;
    for path in files.lines().filter(|l| !l.is_empty()) {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        for violation in find_raw_colour_escapes(path, &text) {
            if total == 0 {
                aprintln!("{} Raw colour escapes found:", p_r("❌"));
            }
            total += 1;
            aprintln!("  {}:{}: {}", p_r(path), violation.line, violation.snippet);
        }
    }

    if total == 0 {
        if !global.is_silent() {
            aprintln!(
                "{} {}",
                p_g("✅"),
                "No raw colour escapes outside the ansi module"
            );
        }
        Ok(true)
    } else {
        if !global.is_silent() {
            aprintln!();
            aprintln!(
                "{} Add a slot in crates/core/src/ansi.rs and a theme key instead",
                p_b("Tip:")
            );
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROD: &str = "crates/ralph/src/render.rs";

    fn hits(path: &str, text: &str) -> Vec<usize> {
        find_raw_colour_escapes(path, text)
            .iter()
            .map(|v| v.line)
            .collect()
    }

    #[test]
    fn flags_colour_literal() {
        let text = "fn a() {\n    let s = \"\\x1b[31mred\";\n}\n";
        let found = find_raw_colour_escapes(PROD, text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].line, 2);
        assert!(found[0].snippet.contains("\\x1b[31m"));
    }

    #[test]
    fn flags_combined_parameters() {
        assert_eq!(hits(PROD, "let s = \"\\x1b[1;31m\";\n"), vec![1]);
        assert_eq!(hits(PROD, "let s = \"\\x1b[92m\";\n"), vec![1]);
        assert_eq!(hits(PROD, "let s = \"\\x1b[107m\";\n"), vec![1]);
    }

    #[test]
    fn allows_resets_and_attributes() {
        let text = "let a = \"\\x1b[0m\";\nlet b = \"\\x1b[1m\";\nlet c = \"\\x1b[2;3;4m\";\nlet d = \"\\x1b[22m\\x1b[24m\\x1b[39m\\x1b[49m\";\n";
        assert!(hits(PROD, text).is_empty());
    }

    #[test]
    fn flags_unicode_escape_form() {
        assert_eq!(hits(PROD, "let s = \"\\u{1b}[34mblue\";\n"), vec![1]);
    }

    #[test]
    fn flags_extended_colours() {
        assert_eq!(hits(PROD, "let s = \"\\x1b[38;5;208m\";\n"), vec![1]);
        assert_eq!(hits(PROD, "let s = \"\\x1b[48;2;1;2;3m\";\n"), vec![1]);
        assert_eq!(
            hits(PROD, "let s = format!(\"\\x1b[38;5;{}m\", n);\n"),
            vec![1]
        );
    }

    #[test]
    fn skips_cfg_test_module_only() {
        let text = "fn a() {}\n#[cfg(test)]\nmod tests {\n    fn t() {\n        let s = \"\\x1b[31m\";\n    }\n}\nfn b() { let s = \"\\x1b[32m\"; }\n";
        assert_eq!(hits(PROD, text), vec![8]);
    }

    #[test]
    fn skips_comment_lines() {
        let text = "/// assert!(r.contains(\"\\x1b[90m\"));\n// \"\\x1b[31m\"\n";
        assert!(hits(PROD, text).is_empty());
    }

    #[test]
    fn skips_allowlisted_files() {
        let text = "let s = \"\\x1b[31m\";\n";
        assert!(hits("crates/core/src/ansi.rs", text).is_empty());
        assert!(hits("crates/core/src/kitty.rs", text).is_empty());
    }

    #[test]
    fn skips_test_files_and_vendor() {
        let text = "let s = \"\\x1b[31m\";\n";
        assert!(hits("crates/ralph/src/tests.rs", text).is_empty());
        assert!(hits("crates/ralph/tests/render.rs", text).is_empty());
        assert!(hits("vendor/x/src/lib.rs", text).is_empty());
        assert!(hits("xtask/src/lint/mod.rs", text).is_empty());
    }

    #[test]
    fn test_module_with_brace_char_literals_and_raw_strings_does_not_hide_production() {
        let text = concat!(
            "#[cfg(test)]\n",
            "mod tests {\n",
            "    fn t() {\n",
            "        let open = '{';\n",
            "        let quote = '\"';\n",
            "        let raw = r#\"\n",
            "{ { {\n",
            "\"#;\n",
            "        let esc = \"\\x1b[31m\";\n",
            "    }\n",
            "}\n",
            "fn prod() { let s = \"\\x1b[32m\"; }\n",
        );
        assert_eq!(hits(PROD, text), vec![12]);
    }

    fn delta(line: &str) -> i64 {
        scan_line(Lex::Code, line).1.delta
    }

    #[test]
    fn scan_ignores_braces_in_strings_chars_and_comments() {
        assert_eq!(delta("let a = \"{\"; let b = '{'; let c = '\"'; {"), 1);
        assert_eq!(delta("let a = '\\'' ; }"), -1);
        assert_eq!(delta("fn f<'a>(x: &'a str) {"), 1);
        assert_eq!(delta("// {{{"), 0);
        assert_eq!(delta("{ /* } } */ }"), 0);
        assert_eq!(delta("let s = \"\\\"{\";"), 0);
    }

    #[test]
    fn scan_carries_raw_string_and_block_state_across_lines() {
        let (state, _) = scan_line(Lex::Code, "let s = r##\"{");
        assert_eq!(state, Lex::Raw(2));
        let (state, braces) = scan_line(state, "\"# } \"##; {");
        assert_eq!(state, Lex::Code);
        assert_eq!(braces.delta, 1);
        let (state, _) = scan_line(Lex::Code, "/* { /* {");
        assert_eq!(state, Lex::Block(2));
        let (state, braces) = scan_line(state, "*/ } */ {");
        assert_eq!(state, Lex::Code);
        assert_eq!(braces.delta, 1);
        let (state, _) = scan_line(Lex::Code, "let s = \"abc");
        assert_eq!(state, Lex::Str);
    }

    #[test]
    fn byte_raw_strings_hide_their_braces() {
        assert_eq!(delta("let s = br\"{\"; {"), 1);
    }

    #[test]
    fn flags_every_escape_spelling() {
        assert_eq!(hits(PROD, "let s = \"\\x1B[31m\";\n"), vec![1]);
        assert_eq!(hits(PROD, "let s = \"\\033[31m\";\n"), vec![1]);
        assert_eq!(hits(PROD, "let s = \"\\u{001b}[31m\";\n"), vec![1]);
        assert_eq!(hits(PROD, "let s = \"\\u{1B}[31m\";\n"), vec![1]);
        assert_eq!(hits(PROD, "let s = \"\\u{00001B}[31m\";\n"), vec![1]);
    }

    #[test]
    fn other_spellings_keep_the_attribute_exemption() {
        let text = "let a = \"\\033[0m\";\nlet b = \"\\x1B[1m\";\nlet c = \"\\u{001b}[22m\";\n";
        assert!(hits(PROD, text).is_empty());
    }

    #[test]
    fn skips_files_named_tests_suffix() {
        let text = "let s = \"\\x1b[31m\";\n";
        assert!(hits("crates/ralph/src/render_tests.rs", text).is_empty());
    }

    #[test]
    fn escape_prefix_len_measures_each_spelling() {
        assert_eq!(escape_prefix_len("x1B[31m"), Some(4));
        assert_eq!(escape_prefix_len("033[31m"), Some(4));
        assert_eq!(escape_prefix_len("u{1b}[31m"), Some(6));
        assert_eq!(escape_prefix_len("u{001B}[31m"), Some(8));
        assert_eq!(escape_prefix_len("u{1b}x"), None);
        assert_eq!(escape_prefix_len("u{0000001b}["), None);
        assert_eq!(escape_prefix_len("u{2b}["), None);
        assert_eq!(escape_prefix_len("n[31m"), None);
    }
}
