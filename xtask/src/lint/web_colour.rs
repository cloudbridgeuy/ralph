use crate::prelude::*;

const ASSET_DIR: &str = "crates/ralph_api/assets";

#[allow(dead_code, reason = "stub: the rules land in the next commit")]
#[derive(Debug, PartialEq, Eq)]
pub struct WebViolation {
    pub line: usize,
    pub snippet: String,
    pub reason: String,
}

#[allow(dead_code, reason = "stub: the rules land in the next commit")]
fn find_web_violations(_path: &str, _text: &str) -> Vec<WebViolation> {
    Vec::new()
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
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        for violation in find_web_violations(path, &text) {
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
    #[ignore = "red until the rules land in the next commit"]
    fn flags_hex_colours_of_every_length() {
        let text = "a { color: #fff; }\nb { color: #ffff; }\nc { color: #0c0c0c; }\nd { color: #0c0c0cff; }\n";
        assert_eq!(lines(CSS, text), vec![1, 2, 3, 4]);
    }

    #[test]
    #[ignore = "red until the rules land in the next commit"]
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
            assert_eq!(lines(CSS, &text), vec![1], "{func}");
        }
    }

    #[test]
    #[ignore = "red until the rules land in the next commit"]
    fn flags_named_colours_in_colour_properties() {
        let text = "a { color: red; }\nb { background: Black; }\nc { border: 1px solid white; }\nd { outline-color: rebeccapurple; }\ne { box-shadow: 0 0 1px navy; }\n";
        assert_eq!(lines(CSS, text), vec![1, 2, 3, 4, 5]);
    }

    #[test]
    #[ignore = "red until the rules land in the next commit"]
    fn flags_a_named_colour_in_a_style_attribute() {
        let text = "<p style=\"color: red\">x</p>\n";
        assert_eq!(lines(HTML, text), vec![1]);
    }

    #[test]
    #[ignore = "red until the rules land in the next commit"]
    fn flags_svg_paint_attributes() {
        let text = "<svg><path fill=\"red\"/><path stroke=\"#fff\"/></svg>\n";
        assert_eq!(lines("crates/ralph_api/assets/i.svg", text), vec![1]);
        assert_eq!(
            find_web_violations("crates/ralph_api/assets/i.svg", text).len(),
            2
        );
    }

    #[test]
    #[ignore = "red until the rules land in the next commit"]
    fn flags_a_variable_outside_the_token_prefix() {
        let text = "a { color: var(--accent); }\nb { color: var(--r-link); }\n";
        assert_eq!(lines(CSS, text), vec![1]);
    }

    #[test]
    #[ignore = "red until the rules land in the next commit"]
    fn accepts_tokens_keywords_and_fallbacks() {
        let text = "a { color: var(--r-ansi-7); background: var(--r-ansi-0); }\nb { border-top: 1px solid var(--r-border, var(--r-ansi-8)); }\nc { background: transparent; color: currentColor; border-color: inherit; }\nd { --r-red-ish: 1; font-family: var(--r-font); }\n";
        assert!(reasons(CSS, text).is_empty());
    }

    #[test]
    #[ignore = "red until the rules land in the next commit"]
    fn ignores_comments_and_fragment_links() {
        let text =
            "/* color: #fff; red */\na { color: var(--r-link); } /* rgb(0 0 0) */\n/*\n#fff\n*/\n";
        assert!(reasons(CSS, text).is_empty());
        let html = "<a href=\"#top\">top</a>\n<a href=\"#add\">add</a>\n<meta name=\"color-scheme\" content=\"dark light\">\n";
        assert!(reasons(HTML, html).is_empty());
    }

    #[test]
    #[ignore = "red until the rules land in the next commit"]
    fn flags_a_custom_element_outside_the_primitive_set() {
        let text = "<r-box>a</r-box>\n<r-card>b</r-card>\n<x-thing></x-thing>\n";
        assert_eq!(lines(HTML, text), vec![2, 2, 3, 3]);
    }

    #[test]
    #[ignore = "red until the rules land in the next commit"]
    fn flags_a_custom_element_defined_outside_the_primitive_set() {
        let text =
            "customElements.define(\"r-card\", Card);\ncustomElements.define('r-box', Box);\n";
        assert_eq!(lines("crates/ralph_api/assets/app.js", text), vec![1]);
    }

    #[test]
    #[ignore = "red until the rules land in the next commit"]
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
    #[ignore = "red until the rules land in the next commit"]
    fn reports_a_reason_and_the_trimmed_line() {
        let found = find_web_violations(CSS, "  a { color: #fff; }  \n");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].snippet, "a { color: #fff; }");
        assert!(found[0].reason.contains("hex"), "{}", found[0].reason);
    }
}
