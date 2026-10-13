const LABEL_START: char = '\u{200B}';
const LABEL_END: char = '\u{200C}';
const URL_START: char = '\u{200D}';
const URL_END: char = '\u{2060}';
const MARKERS: [char; 4] = [LABEL_START, LABEL_END, URL_START, URL_END];

const LINK_SYMBOL: &str = "↗";
const LABEL_STYLE: &str = "\x1b[4;94m";
const LABEL_STYLE_OFF: &str = "\x1b[24m";
const URL_STYLE: &str = "\x1b[96m";
const COLOR_OFF: &str = "\x1b[39m";

pub fn mark_links(text: &str) -> String {
    let chars: Vec<char> = text.chars().filter(|ch| !MARKERS.contains(ch)).collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' => {
                out.extend(&chars[i..(i + 2).min(chars.len())]);
                i += 2;
            }
            '`' => {
                let end = code_span_end(&chars, i);
                out.extend(&chars[i..end]);
                i = end;
            }
            '[' if i == 0 || chars[i - 1] != '!' => match parse_link(&chars, i) {
                Some(link) => {
                    out.push(LABEL_START);
                    out.extend(&chars[link.label.0..link.label.1]);
                    out.push(LABEL_END);
                    out.push(' ');
                    out.push_str(LINK_SYMBOL);
                    out.push(' ');
                    out.push(URL_START);
                    out.extend(&chars[link.url.0..link.url.1]);
                    out.push(URL_END);
                    i = link.end;
                }
                None => {
                    out.push('[');
                    i += 1;
                }
            },
            ch => {
                out.push(ch);
                i += 1;
            }
        }
    }
    out
}

pub fn paint_links(rendered: &str) -> String {
    let mut out = String::with_capacity(rendered.len());
    for ch in rendered.chars() {
        match ch {
            LABEL_START => out.push_str(LABEL_STYLE),
            LABEL_END => out.push_str(LABEL_STYLE_OFF),
            URL_START => out.push_str(URL_STYLE),
            URL_END => out.push_str(COLOR_OFF),
            _ => out.push(ch),
        }
    }
    out
}

struct Link {
    label: (usize, usize),
    url: (usize, usize),
    end: usize,
}

fn code_span_end(chars: &[char], start: usize) -> usize {
    let ticks = chars[start..].iter().take_while(|&&ch| ch == '`').count();
    let mut i = start + ticks;
    while i < chars.len() {
        if chars[i] == '`' {
            let run = chars[i..].iter().take_while(|&&ch| ch == '`').count();
            if run == ticks {
                return i + run;
            }
            i += run;
        } else {
            i += 1;
        }
    }
    start + ticks
}

fn parse_link(chars: &[char], open: usize) -> Option<Link> {
    let label_start = open + 1;
    let label_len = chars[label_start..]
        .iter()
        .position(|&ch| matches!(ch, ']' | '[' | '\n'))?;
    let label_end = label_start + label_len;
    if label_len == 0 || chars[label_end] != ']' || chars.get(label_end + 1) != Some(&'(') {
        return None;
    }
    let url_start = label_end + 2;
    let mut depth = 0usize;
    for (offset, &ch) in chars[url_start..].iter().enumerate() {
        match ch {
            '(' => depth += 1,
            ')' if depth == 0 => {
                let url_end = url_start + offset;
                return (url_end > url_start).then_some(Link {
                    label: (label_start, label_end),
                    url: (url_start, url_end),
                    end: url_end + 1,
                });
            }
            ')' => depth -= 1,
            ch if ch.is_whitespace() => return None,
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip(text: &str) -> String {
        text.chars().filter(|ch| !MARKERS.contains(ch)).collect()
    }

    #[test]
    fn link_becomes_label_symbol_url_without_brackets() {
        assert_eq!(
            strip(&mark_links("see [docs](https://x.dev/a) now")),
            "see docs ↗ https://x.dev/a now"
        );
    }

    #[test]
    fn link_wrapped_in_parentheses_keeps_outer_text() {
        assert_eq!(
            strip(&mark_links("([link](https://l.app/i#c))")),
            "(link ↗ https://l.app/i#c)"
        );
    }

    #[test]
    fn url_with_balanced_parentheses_is_kept_whole() {
        assert_eq!(
            strip(&mark_links("[w](https://en.wiki/A_(b))")),
            "w ↗ https://en.wiki/A_(b)"
        );
    }

    #[test]
    fn non_links_stay_literal() {
        for text in [
            "[a] (b)",
            "[](u)",
            "[a]()",
            "[a](b c)",
            "![img](u)",
            "\\[a](u)",
            "`[a](u)`",
            "``x `[a](u)` y``",
            "[open",
            "[a\nb](u)",
        ] {
            assert_eq!(mark_links(text), text, "{text}");
        }
    }

    #[test]
    fn markers_in_input_are_dropped() {
        assert_eq!(mark_links("a\u{200B}b\u{2060}"), "ab");
    }

    #[test]
    fn markers_take_no_columns_when_wrapping() {
        let skin = crate::markdown::create_markdown_skin();
        let rendered = skin.text(&mark_links("aa [b](c)"), Some(8)).to_string();
        assert_eq!(strip(&rendered).trim_end(), "aa b ↗ c");
    }

    #[test]
    fn paint_colors_label_and_url_differently() {
        let painted = paint_links(&mark_links("[docs](https://x.dev)"));
        assert_eq!(
            painted,
            format!(
                "{LABEL_STYLE}docs{LABEL_STYLE_OFF} {LINK_SYMBOL} {URL_STYLE}https://x.dev{COLOR_OFF}"
            )
        );
        assert_ne!(LABEL_STYLE, URL_STYLE);
    }
}
