use crate::prelude::*;

const ALLOWLIST: &[&str] = &["crates/core/src/ansi.rs", "crates/core/src/kitty.rs"];

const ESCAPE_PREFIXES: &[&str] = &["\\x1b[", "\\u{1b}["];

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
        if (!is_last && (part == "tests" || part == "vendor")) || (is_last && part == "tests.rs") {
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

fn line_has_colour_escape(line: &str) -> bool {
    ESCAPE_PREFIXES.iter().any(|prefix| {
        line.match_indices(prefix)
            .any(|(at, _)| sgr_sets_colour(&line[at + prefix.len()..]))
    })
}

fn brace_delta(line: &str) -> i64 {
    let mut delta = 0;
    let mut in_string = false;
    let mut escaped = false;
    for c in line.chars() {
        match c {
            _ if escaped => escaped = false,
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            '{' if !in_string => delta += 1,
            '}' if !in_string => delta -= 1,
            _ => {}
        }
    }
    delta
}

fn test_module_lines(lines: &[&str]) -> Vec<bool> {
    let mut skipped = vec![false; lines.len()];
    let mut index = 0;
    while index < lines.len() {
        if lines[index].trim() != "#[cfg(test)]" {
            index += 1;
            continue;
        }
        let mut depth = 0i64;
        let mut opened = false;
        let mut end = index;
        for (offset, line) in lines[index..].iter().enumerate() {
            end = index + offset;
            depth += brace_delta(line);
            opened |= line.contains('{');
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
}
