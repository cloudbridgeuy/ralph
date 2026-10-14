const LABEL_START: char = '\u{200B}';
const LABEL_END: char = '\u{200C}';
const URL_START: char = '\u{200D}';
const URL_END: char = '\u{2060}';
const MEMORY_START: char = '\u{2061}';
const MEMORY_END: char = '\u{2062}';
const ADD_START: char = '\u{2063}';
const DEL_START: char = '\u{2064}';
const DIFF_END: char = '\u{FEFF}';
const DIFF_CODE_END: char = '\u{200E}';
const MARKERS: [char; 10] = [
    LABEL_START,
    LABEL_END,
    URL_START,
    URL_END,
    MEMORY_START,
    MEMORY_END,
    ADD_START,
    DEL_START,
    DIFF_END,
    DIFF_CODE_END,
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
                push_marked_diffs(&chars[i..end], i, &chars, &mut out);
                i = end;
            }
            '+' | '-' | '\u{2212}' | '(' => match diff_stat(&chars, i) {
                Some(stat) => {
                    push_diff_stat(&chars, i, &stat, DIFF_END, &mut out);
                    i = stat.end;
                }
                None => {
                    out.push(chars[i]);
                    i += 1;
                }
            },
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
    let add = fg(palette.done);
    let del = fg(palette.blocked);
    let code_restore = match palette.inline_code {
        Some(_) => fg(palette.inline_code),
        None => COLOR_OFF.to_string(),
    };
    let mut out = String::with_capacity(rendered.len());
    let mut open: Option<&str> = None;
    for ch in rendered.chars() {
        match ch {
            LABEL_START => open = Some(&label),
            URL_START => open = Some(&url),
            MEMORY_START => open = Some(&memory),
            ADD_START => open = Some(&add),
            DEL_START => open = Some(&del),
            LABEL_END | URL_END | MEMORY_END | DIFF_END | DIFF_CODE_END => open = None,
            _ => {}
        }
        match ch {
            LABEL_START | URL_START | MEMORY_START | ADD_START | DEL_START => {
                out.push_str(open.unwrap_or_default())
            }
            LABEL_END => out.push_str(LABEL_STYLE_OFF),
            URL_END | MEMORY_END | DIFF_END => out.push_str(COLOR_OFF),
            DIFF_CODE_END => out.push_str(&code_restore),
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

fn push_marked_diffs(span: &[char], offset: usize, chars: &[char], out: &mut String) {
    let mut j = 0;
    while j < span.len() {
        let at = offset + j;
        let stat = match span[j] {
            '+' | '-' | '\u{2212}' | '(' => {
                diff_stat(chars, at).filter(|stat| stat.end <= offset + span.len())
            }
            _ => None,
        };
        match stat {
            Some(stat) => {
                push_diff_stat(chars, at, &stat, DIFF_CODE_END, out);
                j = stat.end - offset;
            }
            None => {
                out.push(span[j]);
                j += 1;
            }
        }
    }
}

struct DiffStat {
    parts: Vec<(usize, usize)>,
    end: usize,
}

fn push_diff_stat(chars: &[char], start: usize, stat: &DiffStat, close: char, out: &mut String) {
    let mut at = start;
    for &(part_start, part_end) in &stat.parts {
        out.extend(&chars[at..part_start]);
        out.push(if chars[part_start] == '+' {
            ADD_START
        } else {
            DEL_START
        });
        out.extend(&chars[part_start..part_end]);
        out.push(close);
        at = part_end;
    }
    out.extend(&chars[at..stat.end]);
}

fn diff_stat(chars: &[char], start: usize) -> Option<DiffStat> {
    let starts_token = start == 0 || chars[start - 1].is_whitespace();
    if !starts_token || is_table_or_heading_line(chars, start) {
        return None;
    }
    let ends_token = |end: usize| chars.get(end).is_none_or(|ch| ch.is_whitespace());
    signed_number_end(chars, start)
        .filter(|&end| ends_token(end))
        .map(|end| DiffStat {
            parts: vec![(start, end)],
            end,
        })
        .or_else(|| {
            diff_pair(chars, start).filter(|stat| {
                ends_token(stat.end)
                    || matches!(chars.get(stat.end), Some('.' | ',' | ';' | ':'))
                        && ends_token(stat.end + 1)
            })
        })
}

fn signed_number_end(chars: &[char], sign: usize) -> Option<usize> {
    if !matches!(chars.get(sign), Some('+' | '-' | '\u{2212}')) {
        return None;
    }
    let digits = digit_run(chars, sign + 1);
    (digits > 0).then_some(sign + 1 + digits)
}

fn diff_pair(chars: &[char], start: usize) -> Option<DiffStat> {
    let wrapped = chars.get(start) == Some(&'(');
    let plus = if wrapped {
        file_count_prefix_end(chars, start + 1)
    } else {
        start
    };
    if chars.get(plus) != Some(&'+') {
        return None;
    }
    let plus_end = signed_number_end(chars, plus)?;
    let minus = plus_end + 1;
    if chars.get(plus_end) != Some(&'/') || !matches!(chars.get(minus), Some('-' | '\u{2212}')) {
        return None;
    }
    let minus_end = signed_number_end(chars, minus)?;
    let end = match wrapped {
        true if chars.get(minus_end) == Some(&')') => minus_end + 1,
        true => return None,
        false => minus_end,
    };
    Some(DiffStat {
        parts: vec![(plus, plus_end), (minus, minus_end)],
        end,
    })
}

fn file_count_prefix_end(chars: &[char], at: usize) -> usize {
    let word = at + digit_run(chars, at);
    let rest = &chars[word.min(chars.len())..];
    let after_word = if word == at {
        None
    } else if rest.starts_with(&[' ', 'f', 'i', 'l', 'e', 's', ',', ' ']) {
        Some(8)
    } else if rest.starts_with(&[' ', 'f', 'i', 'l', 'e', ',', ' ']) {
        Some(7)
    } else {
        None
    };
    after_word.map_or(at, |len| word + len)
}

fn digit_run(chars: &[char], start: usize) -> usize {
    chars[start..]
        .iter()
        .take_while(|ch| ch.is_ascii_digit())
        .count()
}

fn is_table_or_heading_line(chars: &[char], at: usize) -> bool {
    let line_start = chars[..at]
        .iter()
        .rposition(|&ch| ch == '\n')
        .map_or(0, |newline| newline + 1);
    let lead: Vec<char> = chars[line_start..at]
        .iter()
        .copied()
        .skip_while(|ch| ch.is_whitespace())
        .collect();
    let hashes = lead.iter().take_while(|&&ch| ch == '#').count();
    lead.first() == Some(&'|')
        || (1..=6).contains(&hashes) && lead.get(hashes).is_some_and(|ch| ch.is_whitespace())
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

    const GREEN: &str = "\x1b[32m";
    const RED: &str = "\x1b[31m";

    #[test]
    fn paint_colors_signed_numbers_green_and_red() {
        assert_eq!(
            paint(&mark_links("a +5 b -7 c \u{2212}85\n-3\n+4")),
            format!(
                "a {GREEN}+5{COLOR_OFF} b {RED}-7{COLOR_OFF} c {RED}\u{2212}85{COLOR_OFF}\n{RED}-3{COLOR_OFF}\n{GREEN}+4{COLOR_OFF}"
            )
        );
        assert_eq!(
            paint(&mark_links("+204")),
            format!("{GREEN}+204{COLOR_OFF}")
        );
    }

    #[test]
    fn paint_colors_diff_stat_pairs_and_leaves_slash_and_parens_plain() {
        assert_eq!(
            paint(&mark_links("+204/-85")),
            format!("{GREEN}+204{COLOR_OFF}/{RED}-85{COLOR_OFF}")
        );
        assert_eq!(
            paint(&mark_links("x (+204/-85) y")),
            format!("x ({GREEN}+204{COLOR_OFF}/{RED}-85{COLOR_OFF}) y")
        );
        assert_eq!(
            paint(&mark_links("+295/\u{2212}97")),
            format!("{GREEN}+295{COLOR_OFF}/{RED}\u{2212}97{COLOR_OFF}")
        );
    }

    #[test]
    fn paint_colors_file_count_diff_stats() {
        let pair = format!("{GREEN}+377{COLOR_OFF}/{RED}\u{2212}160{COLOR_OFF}");
        assert_eq!(
            paint(&mark_links("(4 files, +377/\u{2212}160)")),
            format!("(4 files, {pair})")
        );
        assert_eq!(
            paint(&mark_links("Changed (4 files, +377/\u{2212}160) today")),
            format!("Changed (4 files, {pair}) today")
        );
        assert_eq!(
            paint(&mark_links("(1 file, +3/-1)")),
            format!("(1 file, {GREEN}+3{COLOR_OFF}/{RED}-1{COLOR_OFF})")
        );
        assert_eq!(
            paint(&mark_links("(4 files, +377/\u{2212}160) extra")),
            format!("(4 files, {pair}) extra")
        );
    }

    #[test]
    fn paint_colors_diff_stat_pairs_before_sentence_punctuation() {
        let pair = format!("{GREEN}+9{COLOR_OFF}/{RED}-2{COLOR_OFF}");
        assert_eq!(
            paint(&mark_links("adds docs (+9/-2).")),
            format!("adds docs ({pair}).")
        );
        assert_eq!(paint(&mark_links("+9/-2, then")), format!("{pair}, then"));
        assert_eq!(
            paint(&mark_links("(4 files, +377/\u{2212}160);")),
            format!("(4 files, {GREEN}+377{COLOR_OFF}/{RED}\u{2212}160{COLOR_OFF});")
        );
        assert_eq!(paint(&mark_links("+9/-2:")), format!("{pair}:"));
    }

    #[test]
    fn signed_numbers_after_hash_words_that_are_not_headings_are_colored() {
        assert_eq!(
            paint(&mark_links("#303 +5\n#tag -2")),
            format!("{MEMORY_STYLE}#303{COLOR_OFF} {GREEN}+5{COLOR_OFF}\n#tag {RED}-2{COLOR_OFF}")
        );
    }

    #[test]
    fn paint_restores_inline_code_color_after_diff_stats() {
        let palette = Palette {
            inline_code: Some(Color::DarkCyan),
            ..Palette::default()
        };
        assert_eq!(
            paint_links(&mark_links("`a +5 b`"), &palette),
            format!("`a {GREEN}+5\x1b[36m b`")
        );
        let plain = Palette {
            inline_code: None,
            ..Palette::default()
        };
        assert_eq!(
            paint_links(&mark_links("`a -2 b`"), &plain),
            format!("`a {RED}-2{COLOR_OFF} b`")
        );
    }

    #[test]
    fn paint_diff_stats_use_palette_colors() {
        let palette = Palette {
            done: Some(Color::DarkBlue),
            blocked: Some(Color::DarkMagenta),
            ..Palette::default()
        };
        assert_eq!(
            paint_links(&mark_links("+1 -2"), &palette),
            format!("\x1b[34m+1{COLOR_OFF} \x1b[35m-2{COLOR_OFF}")
        );
    }

    #[test]
    fn non_diff_stats_stay_literal() {
        for text in [
            "- item",
            "2026-10-13",
            "483-486",
            "-85)",
            "+12px",
            "abc-5",
            "+204/-85,x",
            "(+204/-85).x",
            "(+204/-85)..",
            "-2.",
            "+204/+85",
            "-85/+204",
            "(+204)",
            "+204/-85)",
            "(+204/-85",
            "x+204/-85",
            "(+204/-85)x",
            "(4 files +377/\u{2212}160)",
            "(files, +377/\u{2212}160)",
            "(4 files, +377/\u{2212}160)extra",
            "(4 files, +377/\u{2212}",
            "(4 files, +377/\u{2212} 160)",
            "(4 files,+377/\u{2212}160)",
            "(4 Files, +377/\u{2212}160)",
            "(4  files, +377/\u{2212}160)",
            "\u{2212} 160",
            "(+5",
            "+",
            "-",
            "+5.",
            "+5,",
            "\\+1",
            "| +1 |",
            "  | a | -2",
            "`+5`",
            "## Release 7 has +204 lines",
            "# +1",
            "  ### a -2/x",
        ] {
            assert_eq!(mark_links(text), text, "{text}");
        }
    }

    #[test]
    fn diff_stat_markers_take_no_columns_when_wrapping() {
        let skin = MadSkin::default();
        let rendered = skin.text(&mark_links("aa +12 -34 b"), Some(12)).to_string();
        assert_eq!(strip(&rendered).trim_end(), "aa +12 -34 b");
    }

    #[test]
    fn diff_stat_markers_survive_skin_rendering() {
        let skin = MadSkin::default();
        let rendered = skin
            .text(&mark_links("x +1 -2 `a +3 b`"), Some(40))
            .to_string();
        for marker in [ADD_START, DEL_START, DIFF_END, DIFF_CODE_END] {
            assert!(rendered.contains(marker), "{marker:?}");
        }
    }
}
