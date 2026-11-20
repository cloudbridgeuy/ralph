use super::link::{code_span_end, is_table_or_heading_line};
use super::marker::{
    ADD_START, DEL_START, DIFF_END, MARKERS, MARK_ACCENT, MARK_BAD, MARK_BORDER, MARK_DONE,
    MARK_END, MEMORY_END, MEMORY_START, PILL_ACCENT, PILL_BAD, PILL_BODY, PILL_END, PILL_INFO,
    PILL_OK, PILL_WARN,
};
use super::pill_style::PillStyle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Ok,
    Bad,
    Warn,
    Info,
    Accent,
}

pub struct PillWord {
    pub word: &'static str,
    pub synonyms: &'static [&'static str],
    pub tone: Tone,
}

pub const PILL_WORDS: &[PillWord] = &[
    PillWord {
        word: "Done",
        synonyms: &["Complete", "Fixed"],
        tone: Tone::Ok,
    },
    PillWord {
        word: "Failed",
        synonyms: &["Blocked", "Error"],
        tone: Tone::Bad,
    },
    PillWord {
        word: "Warning",
        synonyms: &["Risk"],
        tone: Tone::Warn,
    },
    PillWord {
        word: "Note",
        synonyms: &["Info"],
        tone: Tone::Info,
    },
    PillWord {
        word: "Question",
        synonyms: &[],
        tone: Tone::Info,
    },
    PillWord {
        word: "Decision",
        synonyms: &[],
        tone: Tone::Accent,
    },
    PillWord {
        word: "Next",
        synonyms: &[],
        tone: Tone::Accent,
    },
];

pub(super) const CAP_LEFT: char = '\u{E0B6}';
pub(super) const CAP_RIGHT: char = '\u{E0B4}';
const NBSP: char = '\u{00A0}';
const MAX_INNER: usize = 32;
const INNER_MARKERS: [char; 5] = [ADD_START, DEL_START, DIFF_END, MEMORY_START, MEMORY_END];

fn inner_text(chars: &[char]) -> Vec<char> {
    chars
        .iter()
        .copied()
        .filter(|ch| !INNER_MARKERS.contains(ch))
        .collect()
}

fn tone_marker(tone: Tone) -> char {
    match tone {
        Tone::Ok => PILL_OK,
        Tone::Bad => PILL_BAD,
        Tone::Warn => PILL_WARN,
        Tone::Info => PILL_INFO,
        Tone::Accent => PILL_ACCENT,
    }
}

fn tone_of(token: &str) -> Option<Tone> {
    PILL_WORDS
        .iter()
        .find(|entry| {
            std::iter::once(&entry.word)
                .chain(entry.synonyms)
                .any(|word| word.eq_ignore_ascii_case(token))
        })
        .map(|entry| entry.tone)
}

struct Pill {
    tone: Tone,
    end: usize,
}

fn parse_pill(chars: &[char], open: usize) -> Option<Pill> {
    if chars.get(open + 1) != Some(&'[') || open > 0 && chars[open - 1] == '[' {
        return None;
    }
    let inner_start = open + 2;
    let inner_len = chars[inner_start..]
        .iter()
        .position(|&ch| matches!(ch, ']' | '[' | '\n'))?;
    let close = inner_start + inner_len;
    let inner = inner_text(&chars[inner_start..close]);
    let closed = chars.get(close) == Some(&']') && chars.get(close + 1) == Some(&']');
    let after = chars.get(close + 2);
    if !closed || matches!(after, Some('(' | ']')) {
        return None;
    }
    if inner.is_empty()
        || inner.len() > MAX_INNER
        || inner
            .iter()
            .any(|ch| MARKERS.contains(ch) || ch.is_control() || matches!(ch, '*' | '_' | '`'))
        || inner[0].is_whitespace()
        || inner[inner.len() - 1].is_whitespace()
    {
        return None;
    }
    let text: String = inner.iter().collect();
    let tone = tone_of(text.split_whitespace().next()?)?;
    Some(Pill {
        tone,
        end: close + 2,
    })
}

fn push_pill(chars: &[char], open: usize, pill: &Pill, style: PillStyle, out: &mut String) {
    out.push(tone_marker(pill.tone));
    if style.round {
        out.push(CAP_LEFT);
        out.push(PILL_BODY);
    }
    out.push(NBSP);
    out.extend(
        inner_text(&chars[open + 2..pill.end - 2])
            .into_iter()
            .map(|ch| if ch == ' ' { NBSP } else { ch }),
    );
    out.push(NBSP);
    out.push(PILL_END);
    if style.round {
        out.push(CAP_RIGHT);
        out.push(MARK_END);
    }
}

fn mark_for(chars: &[char], at: usize) -> Option<(char, usize)> {
    match chars[at] {
        '✓' => Some((MARK_DONE, 1)),
        '✗' => Some((MARK_BAD, 1)),
        '→' => Some((MARK_ACCENT, 1)),
        '├' | '└' if chars.get(at + 1) == Some(&'─') && chars.get(at + 2) == Some(&'─') => {
            Some((MARK_BORDER, 3))
        }
        _ => None,
    }
}

pub fn emphasis_active(flag: bool, color: bool) -> bool {
    flag && color
}

pub fn pill_prompt_rule() -> String {
    let words = PILL_WORDS
        .iter()
        .map(|entry| {
            if entry.synonyms.is_empty() {
                entry.word.to_string()
            } else {
                format!("{} ({})", entry.word, entry.synonyms.join(", "))
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "Status pills: the terminal draws [[Word]] or [[Word detail]] as a coloured pill. \
The text inside the brackets is 1 to {MAX_INNER} characters. Words: {words}. \
Use pills sparingly, only to flag status. Never put them inside code."
    )
}

pub fn mark_pills(text: &str, style: PillStyle) -> String {
    let chars: Vec<char> = text.chars().collect();
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
            ch => {
                let pill = (ch == '[').then(|| parse_pill(&chars, i)).flatten();
                let mark = mark_for(&chars, i);
                let plain =
                    (pill.is_some() || mark.is_some()) && is_table_or_heading_line(&chars, i);
                let (pill, mark) = if plain { (None, None) } else { (pill, mark) };
                match (pill, mark) {
                    (Some(pill), _) => {
                        push_pill(&chars, i, &pill, style, &mut out);
                        i = pill.end;
                    }
                    (None, Some((marker, len))) => {
                        out.push(marker);
                        out.extend(&chars[i..i + len]);
                        out.push(MARK_END);
                        i += len;
                    }
                    (None, None) => {
                        out.push(ch);
                        i += 1;
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::Palette;

    fn mark_pills(text: &str) -> String {
        super::mark_pills(text, PillStyle::default())
    }

    use crate::render::marker::paint_links;

    const NB: char = '\u{00A0}';

    fn pill(start: char, inner: &str) -> String {
        let body: String = inner.replace(' ', &NB.to_string());
        format!("{start}{NB}{body}{NB}{PILL_END}")
    }

    fn strip(text: &str) -> String {
        text.chars().filter(|ch| !MARKERS.contains(ch)).collect()
    }

    #[test]
    fn emphasis_needs_the_flag_and_colour() {
        assert!(emphasis_active(true, true));
        assert!(!emphasis_active(true, false));
        assert!(!emphasis_active(false, true));
        assert!(!emphasis_active(false, false));
    }

    #[test]
    fn valid_pills_become_padded_marker_runs() {
        let cases = [
            ("[[Done]]", pill(PILL_OK, "Done")),
            ("[[Failed]]", pill(PILL_BAD, "Failed")),
            ("[[Warning]]", pill(PILL_WARN, "Warning")),
            ("[[Note]]", pill(PILL_INFO, "Note")),
            ("[[Question]]", pill(PILL_INFO, "Question")),
            ("[[Decision]]", pill(PILL_ACCENT, "Decision")),
            ("[[Next]]", pill(PILL_ACCENT, "Next")),
            ("a [[Done]] b", format!("a {} b", pill(PILL_OK, "Done"))),
        ];
        for (text, want) in cases {
            assert_eq!(mark_pills(text), want, "{text}");
        }
    }

    #[test]
    fn detail_words_join_with_non_breaking_spaces() {
        assert_eq!(
            mark_pills("[[Done 3 of 4 files]]"),
            pill(PILL_OK, "Done 3 of 4 files")
        );
    }

    #[test]
    fn synonyms_and_case_pick_the_tone() {
        assert_eq!(mark_pills("[[fixed]]"), pill(PILL_OK, "fixed"));
        assert_eq!(mark_pills("[[COMPLETE]]"), pill(PILL_OK, "COMPLETE"));
        assert_eq!(mark_pills("[[blocked x]]"), pill(PILL_BAD, "blocked x"));
        assert_eq!(mark_pills("[[Risk]]"), pill(PILL_WARN, "Risk"));
        assert_eq!(mark_pills("[[info]]"), pill(PILL_INFO, "info"));
    }

    #[test]
    fn inner_text_is_one_to_thirty_two_chars() {
        let ok = format!("Done {}", "x".repeat(27));
        assert_eq!(ok.chars().count(), 32);
        assert_eq!(mark_pills(&format!("[[{ok}]]")), pill(PILL_OK, &ok));
        let long = format!("Done {}", "x".repeat(28));
        assert_eq!(long.chars().count(), 33);
        let text = format!("[[{long}]]");
        assert_eq!(mark_pills(&text), text);
    }

    #[test]
    fn near_misses_stay_literal() {
        let cases = [
            "[[]]",
            "[[Unknown]]",
            "[[Done]",
            "[[Done",
            "[Done]",
            "[[Done]]]",
            "[[[Done]]",
            "[[Done [[Next]]]]",
            "[[Do\nne]]",
            "[[Done\n]]",
            "[[ Done]]",
            "[[Done ]]",
            "[[Done]](https://x.y)",
            "\\[[Done]]",
            "`[[Done]]`",
            "``a [[Done]] b``",
            "# Title [[Done]]",
            "| a | [[Done]] |",
            "  | [[Done]] |",
            "plain text",
        ];
        for text in cases {
            assert_eq!(mark_pills(text), text, "{text:?}");
        }
    }

    #[test]
    fn diffstat_and_memory_ref_inside_a_pill_still_make_a_pill() {
        use crate::render::link::mark_links;
        let cases = [
            ("[[Done +3 tests]]", pill(PILL_OK, "Done +3 tests")),
            ("[[Failed -2 files]]", pill(PILL_BAD, "Failed -2 files")),
            ("[[Done #303]]", pill(PILL_OK, "Done #303")),
            ("[[Done +3 -2]]", pill(PILL_OK, "Done +3 -2")),
        ];
        for (text, want) in cases {
            assert_eq!(mark_pills(&mark_links(text)), want, "{text}");
        }
    }

    #[test]
    fn link_markers_inside_a_pill_keep_it_literal() {
        use crate::render::link::mark_links;
        let marked = mark_links("[[Done [a](u)]]");
        assert!(!mark_pills(&marked).contains(PILL_OK));
    }

    #[test]
    fn style_characters_in_the_detail_keep_the_pill_literal() {
        for text in [
            "[[Done *x*]]",
            "[[Done a_b]]",
            "[[Done `x`]]",
            "[[Done **x]]",
        ] {
            assert_eq!(mark_pills(text), text, "{text}");
        }
    }

    #[test]
    fn nested_pill_inside_literal_outer_still_pills_the_inner() {
        let got = mark_pills("[[Done [[Next]]");
        assert_eq!(got, format!("[[Done {}", pill(PILL_ACCENT, "Next")));
    }

    #[test]
    fn pills_are_skipped_only_on_table_and_heading_lines() {
        let got = mark_pills("# H [[Done]]\n[[Done]]\n| [[Done]] |");
        assert_eq!(
            got,
            format!("# H [[Done]]\n{}\n| [[Done]] |", pill(PILL_OK, "Done"))
        );
    }

    #[test]
    fn pill_after_a_closed_code_span_is_marked() {
        let got = mark_pills("`x` [[Done]]");
        assert_eq!(got, format!("`x` {}", pill(PILL_OK, "Done")));
    }

    #[test]
    fn marks_get_marker_pairs() {
        assert_eq!(mark_pills("✓"), format!("{MARK_DONE}✓{MARK_END}"));
        assert_eq!(mark_pills("✗"), format!("{MARK_BAD}✗{MARK_END}"));
        assert_eq!(mark_pills("→"), format!("{MARK_ACCENT}→{MARK_END}"));
        assert_eq!(mark_pills("├── a"), format!("{MARK_BORDER}├──{MARK_END} a"));
        assert_eq!(mark_pills("└── a"), format!("{MARK_BORDER}└──{MARK_END} a"));
    }

    #[test]
    fn marks_are_skipped_in_code_spans_tables_and_headings() {
        for text in ["`✓ → ✗`", "| ✓ |", "# ✓ done", "\\✓"] {
            assert_eq!(mark_pills(text), text, "{text:?}");
        }
    }

    #[test]
    fn lone_tree_stem_stays_literal() {
        assert_eq!(mark_pills("├─ a"), "├─ a");
    }

    #[test]
    fn markers_round_trip_to_the_original_visible_text() {
        let text = "a [[Done 1]] ✓ b → c [[Nope]] ├── x";
        let marked = mark_pills(text);
        assert_eq!(
            strip(&marked).replace(NB, " "),
            "a  Done 1  ✓ b → c [[Nope]] ├── x"
        );
    }

    #[test]
    fn paint_uses_the_tone_background_and_bold() {
        let palette = Palette::default();
        let cases = [
            (PILL_OK, "\x1b[42m"),
            (PILL_BAD, "\x1b[41m"),
            (PILL_WARN, "\x1b[43m"),
            (PILL_INFO, "\x1b[104m"),
            (PILL_ACCENT, "\x1b[104m"),
        ];
        for (start, background) in cases {
            let painted = paint_links(&pill(start, "Done"), &palette);
            assert!(painted.starts_with("\x1b[1m"), "{painted:?}");
            assert!(painted.contains(background), "{painted:?}");
            assert!(painted.ends_with("\x1b[22m\x1b[39m\x1b[49m"), "{painted:?}");
        }
    }

    #[test]
    fn paint_colours_marks_with_palette_slots() {
        let palette = Palette::default();
        let painted = paint_links(&mark_pills("✓ ✗ → ├──"), &palette);
        assert!(painted.contains("\x1b[32m✓\x1b[39m"), "{painted:?}");
        assert!(painted.contains("\x1b[31m✗\x1b[39m"), "{painted:?}");
        assert!(painted.contains("\x1b[94m→\x1b[39m"), "{painted:?}");
        assert!(painted.contains("\x1b[34m├──\x1b[39m"), "{painted:?}");
    }

    #[test]
    fn marked_marks_in_code_spans_paint_nothing() {
        let painted = paint_links(&mark_pills("`✓`"), &Palette::default());
        assert_eq!(painted, "`✓`");
    }

    #[test]
    fn marker_table_has_every_pill_marker() {
        for marker in [
            PILL_OK,
            PILL_BAD,
            PILL_WARN,
            PILL_INFO,
            PILL_ACCENT,
            PILL_END,
        ] {
            assert!(MARKERS.contains(&marker));
        }
    }

    const CAP_L: char = '\u{E0B6}';
    const CAP_R: char = '\u{E0B4}';

    fn round() -> PillStyle {
        PillStyle {
            round: true,
            icons: false,
        }
    }

    fn round_pill(start: char, inner: &str) -> String {
        let body: String = inner.replace(' ', &NB.to_string());
        format!("{start}{CAP_L}{PILL_BODY}{NB}{body}{NB}{PILL_END}{CAP_R}{MARK_END}")
    }

    #[test]
    fn round_pills_get_caps_and_a_body_marker() {
        let cases = [
            ("[[Done]]", round_pill(PILL_OK, "Done")),
            ("[[Failed x y]]", round_pill(PILL_BAD, "Failed x y")),
            (
                "a [[Next]] b",
                format!("a {} b", round_pill(PILL_ACCENT, "Next")),
            ),
        ];
        for (text, want) in cases {
            assert_eq!(super::mark_pills(text, round()), want, "{text}");
        }
    }

    #[test]
    fn square_pills_ignore_the_caps() {
        assert_eq!(
            super::mark_pills("[[Done]]", PillStyle::default()),
            pill(PILL_OK, "Done")
        );
    }

    #[test]
    fn round_mode_leaves_marks_and_near_misses_alone() {
        assert_eq!(
            super::mark_pills("✓ [[Nope]]", round()),
            format!("{MARK_DONE}✓{MARK_END} [[Nope]]")
        );
    }

    #[test]
    fn body_marker_is_in_the_marker_table() {
        assert!(MARKERS.contains(&PILL_BODY));
    }

    #[test]
    fn paint_round_pill_uses_tone_fg_caps_around_a_styled_body() {
        let palette = Palette::default();
        let painted = paint_links(&round_pill(PILL_OK, "Done"), &palette);
        let tone = crate::ansi::fg(palette.pill_ok);
        let body = "\x1b[1m\x1b[42m\x1b[97m";
        let want =
            format!("{tone}{CAP_L}{body}{NB}Done{NB}\x1b[22m\x1b[39m\x1b[49m{tone}{CAP_R}\x1b[39m");
        assert_eq!(painted, want);
    }

    #[test]
    fn paint_square_pill_after_a_round_pill_stays_square() {
        let palette = Palette::default();
        let text = format!(
            "{}{}",
            round_pill(PILL_OK, "Done"),
            pill(PILL_BAD, "Failed")
        );
        let painted = paint_links(&text, &palette);
        assert!(painted.ends_with("\x1b[22m\x1b[39m\x1b[49m"), "{painted:?}");
        assert!(painted.contains("\x1b[1m\x1b[41m"), "{painted:?}");
    }

    #[test]
    fn prompt_rule_lists_every_word_and_synonym_and_the_limit() {
        let rule = pill_prompt_rule();
        for entry in PILL_WORDS {
            assert!(rule.contains(entry.word), "{}", entry.word);
            for synonym in entry.synonyms {
                assert!(rule.contains(synonym), "{synonym}");
            }
        }
        assert!(rule.contains("[[Word]]"));
        assert!(rule.contains("[[Word detail]]"));
        assert!(rule.contains(&MAX_INNER.to_string()));
    }
}
