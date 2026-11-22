use crate::prelude::*;

const ASSET_DIR: &str = "crates/ralph_api/assets";

const PRIMITIVES: &[&str] = &[
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

const COLOUR_FUNCTIONS: &[&str] = &[
    "rgb",
    "rgba",
    "hsl",
    "hsla",
    "hwb",
    "lab",
    "lch",
    "oklab",
    "oklch",
    "color",
    "color-mix",
];

const PAINT_ATTRIBUTES: &[&str] = &["fill", "stroke", "stop-color", "flood-color", "color"];

const NAMED_COLOURS: &[&str] = &[
    "aliceblue",
    "antiquewhite",
    "aqua",
    "aquamarine",
    "azure",
    "beige",
    "bisque",
    "black",
    "blanchedalmond",
    "blue",
    "blueviolet",
    "brown",
    "burlywood",
    "cadetblue",
    "chartreuse",
    "chocolate",
    "coral",
    "cornflowerblue",
    "cornsilk",
    "crimson",
    "cyan",
    "darkblue",
    "darkcyan",
    "darkgoldenrod",
    "darkgray",
    "darkgreen",
    "darkgrey",
    "darkkhaki",
    "darkmagenta",
    "darkolivegreen",
    "darkorange",
    "darkorchid",
    "darkred",
    "darksalmon",
    "darkseagreen",
    "darkslateblue",
    "darkslategray",
    "darkslategrey",
    "darkturquoise",
    "darkviolet",
    "deeppink",
    "deepskyblue",
    "dimgray",
    "dimgrey",
    "dodgerblue",
    "firebrick",
    "floralwhite",
    "forestgreen",
    "fuchsia",
    "gainsboro",
    "ghostwhite",
    "gold",
    "goldenrod",
    "gray",
    "green",
    "greenyellow",
    "grey",
    "honeydew",
    "hotpink",
    "indianred",
    "indigo",
    "ivory",
    "khaki",
    "lavender",
    "lavenderblush",
    "lawngreen",
    "lemonchiffon",
    "lightblue",
    "lightcoral",
    "lightcyan",
    "lightgoldenrodyellow",
    "lightgray",
    "lightgreen",
    "lightgrey",
    "lightpink",
    "lightsalmon",
    "lightseagreen",
    "lightskyblue",
    "lightslategray",
    "lightslategrey",
    "lightsteelblue",
    "lightyellow",
    "lime",
    "limegreen",
    "linen",
    "magenta",
    "maroon",
    "mediumaquamarine",
    "mediumblue",
    "mediumorchid",
    "mediumpurple",
    "mediumseagreen",
    "mediumslateblue",
    "mediumspringgreen",
    "mediumturquoise",
    "mediumvioletred",
    "midnightblue",
    "mintcream",
    "mistyrose",
    "moccasin",
    "navajowhite",
    "navy",
    "oldlace",
    "olive",
    "olivedrab",
    "orange",
    "orangered",
    "orchid",
    "palegoldenrod",
    "palegreen",
    "paleturquoise",
    "palevioletred",
    "papayawhip",
    "peachpuff",
    "peru",
    "pink",
    "plum",
    "powderblue",
    "purple",
    "rebeccapurple",
    "red",
    "rosybrown",
    "royalblue",
    "saddlebrown",
    "salmon",
    "sandybrown",
    "seagreen",
    "seashell",
    "sienna",
    "silver",
    "skyblue",
    "slateblue",
    "slategray",
    "slategrey",
    "snow",
    "springgreen",
    "steelblue",
    "tan",
    "teal",
    "thistle",
    "tomato",
    "turquoise",
    "violet",
    "wheat",
    "white",
    "whitesmoke",
    "yellow",
    "yellowgreen",
];

#[derive(Debug, PartialEq, Eq)]
pub struct WebViolation {
    pub line: usize,
    pub snippet: String,
    pub reason: String,
}

fn strip_comments(line: &str, in_comment: &mut bool) -> String {
    let mut out = String::new();
    let mut rest = line;
    loop {
        if *in_comment {
            match rest.find("*/") {
                Some(at) => {
                    rest = &rest[at + 2..];
                    *in_comment = false;
                }
                None => return out,
            }
        } else {
            match rest.find("/*") {
                Some(at) => {
                    out.push_str(&rest[..at]);
                    out.push(' ');
                    rest = &rest[at + 2..];
                    *in_comment = true;
                }
                None => {
                    out.push_str(rest);
                    return out;
                }
            }
        }
    }
}

fn is_ident_char(c: char) -> bool {
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

fn named_paint_attribute(line: &str) -> bool {
    PAINT_ATTRIBUTES.iter().any(|attr| {
        line.match_indices(&format!("{attr}=\""))
            .any(|(at, found)| {
                let before_ok = line[..at]
                    .chars()
                    .next_back()
                    .is_none_or(|c| !is_ident_char(c));
                let value = &line[at + found.len()..];
                let value = value.split('"').next().unwrap_or("");
                before_ok && value_has_named_colour(value)
            })
    })
}

fn paint_attribute_hex(line: &str) -> bool {
    PAINT_ATTRIBUTES.iter().any(|attr| {
        line.match_indices(&format!("{attr}=\""))
            .any(|(at, found)| {
                let before_ok = line[..at]
                    .chars()
                    .next_back()
                    .is_none_or(|c| !is_ident_char(c));
                let value = &line[at + found.len()..];
                before_ok && hex_colour(value.split('"').next().unwrap_or(""))
            })
    })
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

fn line_reasons(path: &str, line: &str) -> Vec<String> {
    let mut reasons = Vec::new();
    if has_paint_context(path, line) && hex_colour(line) || paint_attribute_hex(line) {
        reasons.push("raw hex colour; use a var(--r-*) token".to_string());
    }
    if colour_function(line) {
        reasons.push("colour function; use a var(--r-*) token".to_string());
    }
    if named_colour_declaration(line) || named_paint_attribute(line) {
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

fn is_checked(path: &str) -> bool {
    ["css", "html", "htm", "svg", "svgz", "js", "mjs"]
        .iter()
        .any(|ext| {
            path.rsplit_once('.')
                .is_some_and(|(_, found)| found == *ext)
        })
}

fn find_web_violations(path: &str, text: &str) -> Vec<WebViolation> {
    if !is_checked(path) {
        return Vec::new();
    }
    let mut in_comment = false;
    text.lines()
        .enumerate()
        .flat_map(|(index, raw)| {
            let line = strip_comments(raw, &mut in_comment);
            line_reasons(path, &line)
                .into_iter()
                .map(|reason| WebViolation {
                    line: index + 1,
                    snippet: raw.trim().to_string(),
                    reason,
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

const BINARY_EXTENSIONS: &[&str] = &["woff", "woff2", "png", "jpg", "jpeg", "gif", "ico", "webp"];

fn is_binary(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, ext)| BINARY_EXTENSIONS.contains(&ext))
}

fn scan_bytes(path: &str, bytes: &[u8]) -> Result<Vec<WebViolation>, String> {
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
        for ext in ["css", "html", "htm", "svg", "svgz", "js", "mjs"] {
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
    fn the_bundled_shell_and_stylesheet_pass() {
        let html = include_str!("../../../crates/ralph_api/assets/index.html");
        let css = include_str!("../../../crates/ralph_api/assets/app.css");
        assert!(reasons(HTML, html).is_empty(), "{:?}", reasons(HTML, html));
        assert!(reasons(CSS, css).is_empty(), "{:?}", reasons(CSS, css));
    }
}
