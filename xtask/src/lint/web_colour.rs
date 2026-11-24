use super::web_colour_names::{COLOUR_FUNCTIONS, NAMED_COLOURS, PAINT_ATTRIBUTES, PRIMITIVES};
use super::web_colour_scan::{statements, strip_comments, without_urls, Terminator};
use crate::prelude::*;

const ASSET_DIR: &str = "crates/ralph_api/assets";

#[derive(Debug, PartialEq, Eq)]
pub struct WebViolation {
    pub line: usize,
    pub snippet: String,
    pub reason: String,
}

pub(super) fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

fn has_paint_context(path: &str, line: &str) -> bool {
    path.ends_with(".css")
        || ["style", "fill", "stroke", "color"]
            .iter()
            .any(|marker| line.contains(marker))
}

fn hex_colour(line: &str) -> bool {
    line.match_indices('#').any(|(at, _)| {
        let digits = line[at + 1..]
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .collect::<String>();
        matches!(digits.len(), 3 | 4 | 6 | 8) && digits.chars().all(|c| c.is_ascii_hexdigit())
    })
}

fn colour_function(line: &str) -> bool {
    line.match_indices('(').any(|(at, _)| {
        let head = &line[..at];
        let name: String = head
            .chars()
            .rev()
            .take_while(|c| is_ident_char(*c))
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        COLOUR_FUNCTIONS.contains(&name.to_ascii_lowercase().as_str())
    })
}

fn foreign_variable(line: &str) -> bool {
    line.match_indices("var(").any(|(at, _)| {
        let name = line[at + 4..].trim_start();
        !name.starts_with("--r-")
    })
}

fn declarations(line: &str) -> impl Iterator<Item = (&str, &str)> {
    line.split([';', '{', '}', '"', '\''])
        .filter_map(|part| part.split_once(':'))
}

fn token_definition(line: &str) -> bool {
    declarations(line).any(|(prop, _)| prop.trim().starts_with("--r-"))
}

fn is_colour_property(prop: &str) -> bool {
    let prop = prop.trim().to_ascii_lowercase();
    prop.starts_with("--")
        || prop == "color"
        || prop == "fill"
        || prop == "stroke"
        || prop.starts_with("background")
        || prop.starts_with("border")
        || prop.starts_with("outline")
        || prop.ends_with("-color")
        || prop.ends_with("shadow")
        || prop.starts_with("text-decoration")
}

fn value_has_named_colour(value: &str) -> bool {
    value
        .split(|c: char| !(c.is_ascii_alphabetic() || c == '-'))
        .any(|word| NAMED_COLOURS.contains(&word.to_ascii_lowercase().as_str()))
}

fn named_colour_declaration(line: &str) -> bool {
    declarations(line)
        .any(|(prop, value)| is_colour_property(prop) && value_has_named_colour(value))
}

fn paint_attribute_values(line: &str) -> Vec<&str> {
    PAINT_ATTRIBUTES
        .iter()
        .flat_map(|attr| {
            let marker = format!("{attr}=\"");
            line.match_indices(&marker)
                .filter(|(at, _)| {
                    line[..*at]
                        .chars()
                        .next_back()
                        .is_none_or(|c| !is_ident_char(c))
                })
                .map(|(at, found)| {
                    let value = &line[at + found.len()..];
                    value.split('"').next().unwrap_or("")
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn named_paint_attribute(line: &str) -> bool {
    paint_attribute_values(line)
        .into_iter()
        .any(value_has_named_colour)
}

fn paint_attribute_hex(line: &str) -> bool {
    paint_attribute_values(line).into_iter().any(hex_colour)
}

fn element_names(line: &str) -> Vec<String> {
    let mut names: Vec<String> = line
        .match_indices('<')
        .filter_map(|(at, _)| {
            let rest = line[at + 1..].trim_start_matches('/');
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect();
            (name.contains('-') && name.starts_with(|c: char| c.is_ascii_alphabetic()))
                .then_some(name)
        })
        .collect();
    names.extend(
        line.match_indices("customElements.define(")
            .filter_map(|(at, found)| {
                let rest = line[at + found.len()..].trim_start();
                let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
                Some(rest[1..].split(quote).next()?.to_string())
            }),
    );
    names
}

fn top_level_arguments(inner: &str) -> Vec<&str> {
    let mut depth = 0usize;
    let mut start = 0;
    let mut parts = Vec::new();
    for (at, c) in inner.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(inner[start..at].trim());
                start = at + 1;
            }
            _ => {}
        }
    }
    parts.push(inner[start..].trim());
    parts
}

fn is_percentage(word: &str) -> bool {
    word.strip_suffix('%').is_some_and(|number| {
        !number.is_empty()
            && number.chars().all(|c| c.is_ascii_digit() || c == '.')
            && number.chars().filter(|c| *c == '.').count() <= 1
    })
}

fn is_token_reference(text: &str) -> bool {
    text.strip_prefix("var(--r-")
        .and_then(|rest| rest.strip_suffix(')'))
        .is_some_and(|name| !name.is_empty() && name.chars().all(is_ident_char))
}

fn is_colour_space(argument: &str) -> bool {
    argument.strip_prefix("in ").is_some_and(|words| {
        words
            .split_whitespace()
            .all(|word| word.chars().all(is_ident_char))
    })
}

fn is_token_colour_argument(argument: &str) -> bool {
    let words: Vec<&str> = argument.split_whitespace().collect();
    match words.as_slice() {
        [token] => is_token_reference(token),
        [percent, token] if is_percentage(percent) => is_token_reference(token),
        [token, percent] if is_percentage(percent) => is_token_reference(token),
        _ => false,
    }
}

fn is_token_only_mix(inner: &str) -> bool {
    let arguments = top_level_arguments(inner);
    let Some((first, rest)) = arguments.split_first() else {
        return false;
    };
    let colours = if is_colour_space(first) {
        rest
    } else {
        &arguments[..]
    };
    !colours.is_empty() && colours.iter().all(|a| is_token_colour_argument(a))
}

fn matching_close(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (at, c) in text[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + at);
                }
            }
            _ => {}
        }
    }
    None
}

fn without_token_mixes(line: &str) -> String {
    const NAME: &str = "color-mix(";
    let mut out = String::new();
    let mut rest = line;
    while let Some(at) = rest.find(NAME) {
        let boundary = rest[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !is_ident_char(c));
        let open = at + NAME.len() - 1;
        match matching_close(rest, open) {
            Some(close) if boundary && is_token_only_mix(&rest[open + 1..close]) => {
                out.push_str(&rest[..at]);
                rest = &rest[close + 1..];
            }
            _ => {
                out.push_str(&rest[..at + NAME.len()]);
                rest = &rest[at + NAME.len()..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn text_reasons(path: &str, raw: &str, selector: bool) -> Vec<String> {
    let text = without_token_mixes(raw);
    let line = text.as_str();
    let paint = !selector;
    let mut reasons = Vec::new();
    if paint && (has_paint_context(path, line) && hex_colour(line) || paint_attribute_hex(line)) {
        reasons.push("raw hex colour; use a var(--r-*) token".to_string());
    }
    if colour_function(line) {
        reasons.push("colour function; use a var(--r-*) token".to_string());
    }
    if paint && (named_colour_declaration(line) || named_paint_attribute(line)) {
        reasons.push("named colour; use a var(--r-*) token".to_string());
    }
    if foreign_variable(line) {
        reasons.push("variable outside the --r-* token prefix".to_string());
    }
    if token_definition(line) {
        reasons.push("token definition; --r-* tokens come only from /tokens.css".to_string());
    }
    for name in element_names(line) {
        if !PRIMITIVES.contains(&name.as_str()) {
            reasons.push(format!("custom element {name} is not a primitive"));
        }
    }
    reasons
}

fn is_script(path: &str) -> bool {
    path.ends_with(".js") || path.ends_with(".mjs")
}

fn is_checked(path: &str) -> bool {
    ["css", "html", "htm", "svg", "js", "mjs"]
        .iter()
        .any(|ext| {
            path.rsplit_once('.')
                .is_some_and(|(_, found)| found == *ext)
        })
}

fn push_reasons(found: &mut Vec<WebViolation>, lines: &[&str], line: usize, reasons: Vec<String>) {
    let earlier = found.len();
    for reason in reasons {
        if !found[..earlier]
            .iter()
            .any(|v| v.line == line && v.reason == reason)
        {
            found.push(WebViolation {
                line,
                snippet: lines
                    .get(line - 1)
                    .map_or(String::new(), |raw| raw.trim().to_string()),
                reason,
            });
        }
    }
}

fn find_web_violations(path: &str, text: &str) -> Vec<WebViolation> {
    if !is_checked(path) {
        return Vec::new();
    }
    let raw_lines: Vec<&str> = text.lines().collect();
    let cleaned = without_urls(&strip_comments(text, is_script(path)));
    let mut found = Vec::new();
    if path.ends_with(".css") {
        for statement in statements(&cleaned) {
            let selector = statement.terminator == Terminator::OpenBrace;
            let reasons = text_reasons(path, statement.text, selector);
            push_reasons(&mut found, &raw_lines, statement.line, reasons);
        }
    } else {
        for (index, line) in cleaned.lines().enumerate() {
            push_reasons(
                &mut found,
                &raw_lines,
                index + 1,
                text_reasons(path, line, false),
            );
        }
    }
    found
}

const BINARY_EXTENSIONS: &[&str] = &["woff", "woff2", "png", "jpg", "jpeg", "gif", "ico", "webp"];

fn is_binary(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, ext)| BINARY_EXTENSIONS.contains(&ext))
}

fn scan_bytes(path: &str, bytes: &[u8]) -> Result<Vec<WebViolation>, String> {
    if path.ends_with(".svgz") {
        return Err("gzipped SVG can't be scanned; use plain .svg".to_string());
    }
    if is_binary(path) {
        return Ok(Vec::new());
    }
    let text = std::str::from_utf8(bytes).map_err(|err| format!("not valid UTF-8: {err}"))?;
    Ok(find_web_violations(path, text))
}

pub async fn run_web_colour_check(global: &crate::Global) -> super::error::Result<bool> {
    if !global.is_silent() {
        aprintln!(
            "{} {}",
            p_b("🔧"),
            p_b("Checking web assets for raw colours...")
        );
    }

    let output = tokio::process::Command::new("git")
        .args(["ls-files", ASSET_DIR])
        .output()
        .await?;

    if !output.status.success() {
        aprintln!("{} Failed to list web assets", p_r("❌"));
        return Ok(false);
    }

    let files = String::from_utf8_lossy(&output.stdout);
    let mut total = 0;
    for path in files.lines().filter(|l| !l.is_empty()) {
        let scanned = std::fs::read(path)
            .map_err(|err| format!("cannot read: {err}"))
            .and_then(|bytes| scan_bytes(path, &bytes));
        let violations = scanned.unwrap_or_else(|reason| {
            vec![WebViolation {
                line: 0,
                snippet: String::new(),
                reason,
            }]
        });
        for violation in violations {
            if total == 0 {
                aprintln!("{} Web asset violations found:", p_r("❌"));
            }
            total += 1;
            aprintln!(
                "  {}:{}: {} ({})",
                p_r(path),
                violation.line,
                violation.snippet,
                violation.reason
            );
        }
    }

    if total == 0 {
        if !global.is_silent() {
            aprintln!(
                "{} {}",
                p_g("✅"),
                "Web assets use only var(--r-*) colours and primitive elements"
            );
        }
        Ok(true)
    } else {
        if !global.is_silent() {
            aprintln!();
            aprintln!(
                "{} Use var(--r-*) tokens and the primitive element set instead",
                p_b("Tip:")
            );
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CSS: &str = "crates/ralph_api/assets/app.css";
    const HTML: &str = "crates/ralph_api/assets/index.html";

    fn reasons(path: &str, text: &str) -> Vec<(usize, String)> {
        find_web_violations(path, text)
            .into_iter()
            .map(|v| (v.line, v.reason))
            .collect()
    }

    fn lines(path: &str, text: &str) -> Vec<usize> {
        reasons(path, text)
            .into_iter()
            .map(|(line, _)| line)
            .collect()
    }

    #[test]
    fn flags_hex_colours_of_every_length() {
        let text = "a { color: #fff; }\nb { color: #ffff; }\nc { color: #0c0c0c; }\nd { color: #0c0c0cff; }\n";
        assert_eq!(lines(CSS, text), vec![1, 2, 3, 4]);
    }

    #[test]
    fn checks_every_web_source_extension() {
        for ext in ["css", "html", "htm", "svg", "js", "mjs"] {
            let path = format!("crates/ralph_api/assets/x.{ext}");
            assert_eq!(lines(&path, "a { color: #fff; }\n"), vec![1], "{ext}");
        }
    }

    #[test]
    fn a_known_binary_asset_is_skipped() {
        let font = [0x77, 0x4f, 0x46, 0x32, 0xff, 0xfe];
        assert!(scan_bytes("crates/ralph_api/assets/fonts/x.woff2", &font)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn a_gzipped_svg_is_refused() {
        for bytes in [&[0x1f, 0x8b, 0x08, 0x00][..], b"<svg/>"] {
            let err = scan_bytes("crates/ralph_api/assets/i.svgz", bytes).unwrap_err();
            assert_eq!(err, "gzipped SVG can't be scanned; use plain .svg");
        }
    }

    #[test]
    fn a_non_utf8_file_is_an_error_not_a_skip() {
        let bytes = [0xff, 0xfe, 0x00];
        for path in [
            "crates/ralph_api/assets/a.css",
            "crates/ralph_api/assets/a.bin",
        ] {
            let err = scan_bytes(path, &bytes).unwrap_err();
            assert!(err.contains("UTF-8"), "{err}");
        }
    }

    #[test]
    fn text_bytes_are_scanned() {
        let found = scan_bytes(CSS, b"a { color: #fff; }\n").unwrap();
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn allows_a_colour_mix_of_tokens_only() {
        for mix in [
            "color-mix(in srgb, var(--r-fg) 60%, var(--r-bg))",
            "color-mix(in oklab, var(--r-ansi-1), var(--r-ansi-2) 30%)",
            "color-mix(in srgb, 40% var(--r-bg), var(--r-fg))",
        ] {
            let text = format!("a {{ color: {mix}; }}\n");
            assert!(lines(CSS, &text).is_empty(), "{mix}");
        }
    }

    #[test]
    fn refuses_a_colour_mix_with_a_raw_colour() {
        for mix in [
            "color-mix(in srgb, var(--r-fg) 60%, red)",
            "color-mix(in srgb, var(--r-fg), #fff)",
            "color-mix(in srgb, rgb(0 0 0), var(--r-bg))",
            "color-mix(in srgb, var(--r-fg), hsl(0 0% 0%))",
            "color-mix(in srgb, var(--r-fg, red), var(--r-bg))",
        ] {
            let text = format!("a {{ color: {mix}; }}\n");
            let mut found = lines(CSS, &text);
            found.dedup();
            assert_eq!(found, vec![1], "{mix}");
        }
    }

    #[test]
    fn refuses_a_colour_mix_with_a_foreign_variable() {
        let text = "a { color: color-mix(in srgb, var(--brand) 60%, var(--r-bg)); }\n";
        let mut found = lines(CSS, text);
        found.dedup();
        assert_eq!(found, vec![1]);
    }

    #[test]
    fn flags_colour_functions() {
        for func in [
            "rgb(0 0 0)",
            "rgba(0,0,0,.5)",
            "hsl(0 0% 0%)",
            "hsla(0,0%,0%,1)",
            "hwb(0 0% 0%)",
            "lab(0 0 0)",
            "lch(0 0 0)",
            "oklab(0 0 0)",
            "oklch(0 0 0)",
            "color(srgb 0 0 0)",
            "color-mix(in srgb, red, blue)",
        ] {
            let text = format!("a {{ background: {func}; }}\n");
            let mut found = lines(CSS, &text);
            found.dedup();
            assert_eq!(found, vec![1], "{func}");
        }
    }

    #[test]
    fn flags_named_colours_in_colour_properties() {
        let text = "a { color: red; }\nb { background: Black; }\nc { border: 1px solid white; }\nd { outline-color: rebeccapurple; }\ne { box-shadow: 0 0 1px navy; }\n";
        assert_eq!(lines(CSS, text), vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn flags_a_named_colour_in_a_style_attribute() {
        let text = "<p style=\"color: red\">x</p>\n";
        assert_eq!(lines(HTML, text), vec![1]);
    }

    #[test]
    fn flags_svg_paint_attributes() {
        let text = "<svg><path fill=\"red\"/><path stroke=\"#fff\"/></svg>\n";
        assert_eq!(lines("crates/ralph_api/assets/i.svg", text), vec![1, 1]);
    }

    #[test]
    fn flags_a_variable_outside_the_token_prefix() {
        let text = "a { color: var(--accent); }\nb { color: var(--r-link); }\n";
        assert_eq!(lines(CSS, text), vec![1]);
    }

    #[test]
    fn flags_a_token_definition_and_a_named_colour_in_a_custom_property() {
        let text = ":root { --r-accent: tomato; }\na { --r-link: var(--r-ansi-4); }\nb { --accent: tomato; }\nc { --accent: var(--r-ansi-4); }\n";
        assert_eq!(lines(CSS, text), vec![1, 1, 2, 3]);
        let found = reasons(CSS, text);
        assert!(found[0].1.contains("named colour"), "{}", found[0].1);
        assert!(found[1].1.contains("token"), "{}", found[1].1);
    }

    #[test]
    fn accepts_tokens_keywords_and_fallbacks() {
        let text = "a { color: var(--r-ansi-7); background: var(--r-ansi-0); }\nb { border-top: 1px solid var(--r-border, var(--r-ansi-8)); }\nc { background: transparent; color: currentColor; border-color: inherit; }\nd { font-family: var(--r-font); --gap: 1px; }\n";
        assert!(reasons(CSS, text).is_empty());
    }

    #[test]
    fn ignores_comments_and_fragment_links() {
        let text =
            "/* color: #fff; red */\na { color: var(--r-link); } /* rgb(0 0 0) */\n/*\n#fff\n*/\n";
        assert!(reasons(CSS, text).is_empty());
        let html = "<a href=\"#top\">top</a>\n<a href=\"#add\">add</a>\n<meta name=\"color-scheme\" content=\"dark light\">\n";
        assert!(reasons(HTML, html).is_empty());
    }

    #[test]
    fn flags_a_custom_element_outside_the_primitive_set() {
        let text = "<r-box>a</r-box>\n<r-card>b</r-card>\n<x-thing></x-thing>\n";
        assert_eq!(lines(HTML, text), vec![2, 2, 3, 3]);
    }

    #[test]
    fn flags_a_custom_element_defined_outside_the_primitive_set() {
        let text =
            "customElements.define(\"r-card\", Card);\ncustomElements.define('r-box', Box);\n";
        assert_eq!(lines("crates/ralph_api/assets/app.js", text), vec![1]);
    }

    #[test]
    fn accepts_every_primitive() {
        let names = [
            "r-box",
            "r-rule",
            "r-table",
            "r-kv",
            "r-tree",
            "r-pill",
            "r-status",
            "r-graph",
            "r-code",
            "r-prose",
            "r-spinner",
            "r-spark",
            "r-empty",
            "r-error",
        ];
        let text: String = names.iter().map(|n| format!("<{n}></{n}>\n")).collect();
        assert!(reasons(HTML, &text).is_empty());
    }

    #[test]
    fn reports_a_reason_and_the_trimmed_line() {
        let found = find_web_violations(CSS, "  a { color: #fff; }  \n");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].snippet, "a { color: #fff; }");
        assert!(found[0].reason.contains("hex"), "{}", found[0].reason);
    }

    #[test]
    fn notices_and_binary_assets_are_not_scanned() {
        let text = "Copyright (c) <the-holder> color: #fff red\n";
        assert!(reasons("crates/ralph_api/assets/fonts/OFL.txt", text).is_empty());
        assert!(reasons("crates/ralph_api/assets/fonts/NOTICE", text).is_empty());
    }

    #[test]
    fn an_id_selector_that_looks_like_hex_is_not_a_colour() {
        for text in [
            "#add {\n  margin: 0;\n}\n",
            "a:hover #add,\n#bee {\n  margin: 0;\n}\n",
            "#add,\n#bee {\n  margin: 0;\n}\n",
            "#add { margin: 0; }\n",
        ] {
            assert!(reasons(CSS, text).is_empty(), "{text}");
        }
    }

    #[test]
    fn a_hex_value_in_a_declaration_is_still_a_colour() {
        for text in [
            "a { color: #add; }\n",
            "#add { color: #add; }\n",
            "a { box-shadow: 0 0 #add, 0 0 #bee; }\n",
        ] {
            assert_eq!(lines(CSS, text), vec![1], "{text}");
        }
    }

    #[test]
    fn the_contents_of_url_are_not_scanned() {
        for value in [
            "url(gold.svg)",
            "url(#add)",
            "url(\"red.png\")",
            "url('blue.png')",
        ] {
            let text = format!("a {{ background: {value}; }}\n");
            assert!(reasons(CSS, &text).is_empty(), "{value}");
        }
    }

    #[test]
    fn a_colour_beside_a_url_is_still_flagged() {
        assert_eq!(lines(CSS, "a { background: red url(x.svg); }\n"), vec![1]);
        assert_eq!(lines(CSS, "a { border: 1px solid gold; }\n"), vec![1]);
    }

    #[test]
    fn a_value_split_across_lines_is_judged_whole() {
        for text in [
            "a {\n  color:\n    red;\n}\n",
            "a {\n  --x:\n    #fff;\n}\n",
            "a {\n  color: color-mix(in srgb, var(--r-fg),\n    red);\n}\n",
            "a {\n  color: var(--r-fg,\n    red);\n}\n",
        ] {
            let found = lines(CSS, text);
            assert!(!found.is_empty(), "{text}");
            assert!(found.iter().all(|line| *line == 2), "{text}: {found:?}");
        }
    }

    #[test]
    fn a_multi_line_value_of_tokens_is_allowed() {
        for text in [
            "a {\n  color: color-mix(in srgb,\n    var(--r-fg) 60%,\n    var(--r-bg));\n}\n",
            "a {\n  color: var(\n    --r-fg);\n}\n",
            "a {\n  box-shadow:\n    0 0 1px var(--r-fg),\n    0 0 2px var(--r-bg);\n}\n",
        ] {
            assert!(reasons(CSS, text).is_empty(), "{text}");
        }
    }

    #[test]
    fn a_multi_line_selector_is_never_a_colour() {
        let text = "a:hover #add,\n#bee,\nred {\n  margin: 0;\n}\n";
        assert!(reasons(CSS, text).is_empty());
    }

    #[test]
    fn two_statements_on_one_line_report_one_reason() {
        let text = "a { color: red; background: blue; }\n";
        assert_eq!(lines(CSS, text), vec![1]);
    }

    #[test]
    fn a_token_definition_is_refused_even_across_lines() {
        for text in [
            ":root { --r-x: var(--r-fg); }\n",
            ":root {\n  --r-x:\n    red;\n}\n",
        ] {
            let found = reasons(CSS, text);
            assert!(
                found
                    .iter()
                    .any(|(_, reason)| reason.contains("token definition")),
                "{text}: {found:?}"
            );
        }
    }

    #[test]
    fn html_and_js_keep_the_per_line_paint_rules() {
        assert_eq!(lines(HTML, "<p style=\"color: #fff\">x</p>\n"), vec![1]);
        assert!(lines(HTML, "<a href=\"#add\">x</a>\n").is_empty());
        let js = "crates/ralph_api/assets/app.js";
        assert_eq!(lines(js, "el.style.color = \"#fff\";\n"), vec![1]);
    }

    #[test]
    fn a_js_line_comment_is_not_scanned() {
        let js = "crates/ralph_api/assets/app.js";
        for text in [
            "// color: #fff\n",
            "x(); // color: red; rgb(0 0 0)\n",
            "// <r-card>\n",
        ] {
            assert!(reasons(js, text).is_empty(), "{text}");
        }
        let mjs = "crates/ralph_api/assets/app.mjs";
        assert!(reasons(mjs, "// style color: #fff\n").is_empty());
    }

    #[test]
    fn a_js_string_with_slashes_is_still_scanned() {
        let js = "crates/ralph_api/assets/app.js";
        assert_eq!(
            lines(js, "u = \"http://x\"; el.style.color = \"#fff\";\n"),
            vec![1]
        );
        assert_eq!(lines(js, "u = '/*'; el.style.color = '#fff';\n"), vec![1]);
        assert_eq!(lines(js, "// note\nel.style.color = \"#fff\";\n"), vec![2]);
    }

    #[test]
    fn a_double_slash_in_css_is_not_a_comment() {
        assert_eq!(lines(CSS, "a { color: red; // x\n}\n"), vec![1]);
        assert_eq!(
            lines(CSS, "a {\n  // color: red;\n  color: blue;\n}\n"),
            vec![3]
        );
        assert_eq!(lines(HTML, "<p style=\"color: red\"> // x\n"), vec![1]);
    }

    #[test]
    fn a_comment_inside_a_multi_line_value_keeps_line_numbers() {
        let text = "a {\n  color: /* x\n y */\n    red;\n}\n";
        assert_eq!(lines(CSS, text), vec![2]);
    }

    #[test]
    fn a_token_definition_in_a_comment_is_ignored_but_one_after_it_is_not() {
        assert!(reasons(CSS, "/* --r-x: 1; */\n").is_empty());
        assert_eq!(lines(CSS, "/* a */ :root { --r-x: 1; }\n"), vec![1]);
    }

    #[test]
    fn an_unclosed_quote_in_url_does_not_hide_the_rest_of_the_file() {
        let text = "a { background: url(\"x; }\nb { color: #fff; }\n";
        assert_eq!(lines(CSS, text), vec![2]);
    }

    #[test]
    fn the_bundled_shell_and_stylesheet_pass() {
        let html = include_str!("../../../crates/ralph_api/assets/index.html");
        let css = include_str!("../../../crates/ralph_api/assets/app.css");
        assert!(reasons(HTML, html).is_empty(), "{:?}", reasons(HTML, html));
        assert!(reasons(CSS, css).is_empty(), "{:?}", reasons(CSS, css));
    }
}
