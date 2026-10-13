const LABEL_START: char = '\u{200B}';
const LABEL_END: char = '\u{200C}';
const URL_START: char = '\u{200D}';
const URL_END: char = '\u{2060}';
const MEMORY_START: char = '\u{2061}';
const MEMORY_END: char = '\u{2062}';
const MARKERS: [char; 6] = [
    LABEL_START,
    LABEL_END,
    URL_START,
    URL_END,
    MEMORY_START,
    MEMORY_END,
];

use crate::ansi::fg;
use crate::markdown::Palette;

const LINK_SYMBOL: &str = "↗";
const UNDERLINE: &str = "\x1b[4m";
const LABEL_STYLE_OFF: &str = "\x1b[24m";
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
            '#' if is_memory_boundary(&chars, i) => match memory_ref_end(&chars, i) {
                Some(end) => {
                    out.push(MEMORY_START);
                    out.extend(&chars[i..end]);
                    out.push(MEMORY_END);
                    i = end;
                }
                None => {
                    out.push('#');
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

pub fn is_marker(ch: char) -> bool {
    MARKERS.contains(&ch)
}

pub fn paint_links(rendered: &str, palette: &Palette) -> String {
    let label = format!("{UNDERLINE}{}", fg(palette.link));
    let url = fg(palette.url);
    let memory = fg(palette.memory_ref);
    let mut out = String::with_capacity(rendered.len());
    let mut open: Option<&str> = None;
    for ch in rendered.chars() {
        match ch {
            LABEL_START => open = Some(&label),
            URL_START => open = Some(&url),
            MEMORY_START => open = Some(&memory),
            LABEL_END | URL_END | MEMORY_END => open = None,
            _ => {}
        }
        match ch {
            LABEL_START | URL_START | MEMORY_START => out.push_str(open.unwrap_or_default()),
            LABEL_END => out.push_str(LABEL_STYLE_OFF),
            URL_END | MEMORY_END => out.push_str(COLOR_OFF),
            '\n' => {
                out.push('\n');
                out.push_str(open.unwrap_or_default());
            }
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

fn is_memory_boundary(chars: &[char], hash: usize) -> bool {
    hash == 0
        || !matches!(chars[hash - 1], '_' | '#' | '&' | '/' | '\\')
            && !chars[hash - 1].is_alphanumeric()
}

fn memory_ref_end(chars: &[char], hash: usize) -> Option<usize> {
    let digits = chars[hash + 1..]
        .iter()
        .take_while(|ch| ch.is_ascii_digit())
        .count();
    let end = hash + 1 + digits;
    let word_continues = chars
        .get(end)
        .is_some_and(|&ch| ch == '_' || ch.is_alphanumeric());
    (digits > 0 && !word_continues).then_some(end)
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

    use termimad::crossterm::style::Color;
    use termimad::MadSkin;

    const LABEL_STYLE: &str = "\x1b[4m\x1b[94m";
    const URL_STYLE: &str = "\x1b[96m";
    const MEMORY_STYLE: &str = "\x1b[95m";

    fn paint(text: &str) -> String {
        paint_links(text, &Palette::default())
    }

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
        let skin = MadSkin::default();
        let rendered = skin.text(&mark_links("aa [b](c)"), Some(8)).to_string();
        assert_eq!(strip(&rendered).trim_end(), "aa b ↗ c");
    }

    fn memory_refs(text: &str) -> Vec<String> {
        let marked = mark_links(text);
        marked
            .split(MEMORY_START)
            .skip(1)
            .filter_map(|rest| rest.split_once(MEMORY_END).map(|(id, _)| id.to_string()))
            .collect()
    }

    #[test]
    fn memory_refs_are_marked_at_word_boundaries() {
        assert_eq!(memory_refs("see #303."), ["#303"]);
        assert_eq!(
            memory_refs("#1 and (#22), #4-#5"),
            ["#1", "#22", "#4", "#5"]
        );
        assert_eq!(memory_refs("line\n#7"), ["#7"]);
    }

    #[test]
    fn non_memory_refs_stay_literal() {
        for text in [
            "# Title",
            "## 12",
            "abc#12",
            "##12",
            "#12abc",
            "#12_x",
            "&#123;",
            "https://x.dev/#12",
            "#",
            "#x",
            "`#303`",
            "``a `#303` b``",
            "\\#303",
            "[see #303](https://x.dev/i#12)",
        ] {
            assert!(memory_refs(text).is_empty(), "{text}");
        }
    }

    #[test]
    fn memory_ref_markers_take_no_columns_when_wrapping() {
        let skin = MadSkin::default();
        let rendered = skin.text(&mark_links("aa #303 b"), Some(8)).to_string();
        assert_eq!(
            strip(&rendered).lines().next().map(str::trim_end),
            Some("aa #303")
        );
    }

    #[test]
    fn paint_reopens_style_after_a_line_break() {
        assert_eq!(
            paint(&mark_links("[a b](https://x.dev/a)").replace(' ', "\n")),
            format!(
                "{LABEL_STYLE}a\n{LABEL_STYLE}b{LABEL_STYLE_OFF}\n{LINK_SYMBOL}\n{URL_STYLE}https://x.dev/a{COLOR_OFF}"
            )
        );
    }

    #[test]
    fn paint_colors_memory_ref_pink() {
        assert_eq!(
            paint(&mark_links("#303")),
            format!("{MEMORY_STYLE}#303{COLOR_OFF}")
        );
    }

    #[test]
    fn paint_colors_label_and_url_differently() {
        let painted = paint(&mark_links("[docs](https://x.dev)"));
        assert_eq!(
            painted,
            format!(
                "{LABEL_STYLE}docs{LABEL_STYLE_OFF} {LINK_SYMBOL} {URL_STYLE}https://x.dev{COLOR_OFF}"
            )
        );
        assert_ne!(LABEL_STYLE, URL_STYLE);
    }

    #[test]
    fn paint_uses_palette_colors() {
        let palette = Palette {
            memory_ref: Some(Color::DarkGreen),
            ..Palette::default()
        };
        assert_eq!(
            paint_links(&mark_links("#7"), &palette),
            format!("\x1b[32m#7{COLOR_OFF}")
        );
    }
}
