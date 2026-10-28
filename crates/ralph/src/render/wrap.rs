use std::ops::Range;
use std::sync::OnceLock;

use termimad::minimad::Compound;
use termimad::{CompositeKind, FmtComposite, FmtLine, FmtText, MadSkin};
use unicode_linebreak::linebreaks;
use unicode_width::UnicodeWidthChar;

use super::link::is_marker;

static WRAP_CAP: OnceLock<usize> = OnceLock::new();

pub fn cap_wrap_width(cap: Option<usize>) {
    if let Some(cap) = cap {
        WRAP_CAP.get_or_init(|| cap);
    }
}

pub fn wrap_width(terminal: usize) -> usize {
    capped_width(terminal, WRAP_CAP.get().copied())
}

fn capped_width(terminal: usize, cap: Option<usize>) -> usize {
    cap.map_or(terminal, |cap| cap.min(terminal))
}

enum Piece<'s> {
    Text(FmtComposite<'s>),
    Table(Vec<FmtLine<'s>>),
}

fn pieces(lines: Vec<FmtLine<'_>>) -> Vec<Piece<'_>> {
    let mut pieces = Vec::new();
    for line in lines {
        match line {
            FmtLine::Normal(composite) => pieces.push(Piece::Text(composite)),
            other => match pieces.last_mut() {
                Some(Piece::Table(run)) => run.push(other),
                _ => pieces.push(Piece::Table(vec![other])),
            },
        }
    }
    pieces
}

pub fn render_prose(skin: &MadSkin, src: &str, width: usize) -> String {
    let mut fitted_tables = pieces(skin.text(src, Some(width)).lines)
        .into_iter()
        .filter_map(|piece| match piece {
            Piece::Table(run) => Some(run),
            Piece::Text(_) => None,
        });
    let lines = pieces(skin.text(src, None).lines)
        .into_iter()
        .flat_map(|piece| match piece {
            Piece::Text(composite) => wrap_composite(composite, width, skin)
                .into_iter()
                .map(FmtLine::Normal)
                .collect(),
            Piece::Table(run) => fitted_tables.next().unwrap_or(run),
        })
        .collect();
    FmtText {
        skin,
        lines,
        width: Some(width),
    }
    .to_string()
}

fn wrap_composite<'s>(
    composite: FmtComposite<'s>,
    width: usize,
    skin: &MadSkin,
) -> Vec<FmtComposite<'s>> {
    let (left, right) = skin.line_style(composite.kind).margins_in(Some(width));
    let available = width.saturating_sub(left + right);
    if composite.kind == CompositeKind::Code || composite.visible_length <= available {
        return vec![composite];
    }
    let follow_up = follow_up_kind(composite.kind);
    let first = available.saturating_sub(skin.visible_composite_length(composite.kind, &[]));
    let rest = available.saturating_sub(skin.visible_composite_length(follow_up, &[]));
    let text: String = composite.compounds.iter().map(|c| c.src).collect();
    break_lines(&text, first, rest)
        .iter()
        .enumerate()
        .map(|(index, range)| {
            let mut line = FmtComposite {
                kind: if index == 0 {
                    composite.kind
                } else {
                    follow_up
                },
                compounds: slice_compounds(&composite.compounds, range),
                visible_length: 0,
                spacing: composite.spacing,
            };
            line.recompute_width(skin);
            line
        })
        .collect()
}

fn follow_up_kind(kind: CompositeKind) -> CompositeKind {
    match kind {
        CompositeKind::ListItem(depth) => CompositeKind::ListItemFollowUp(depth),
        CompositeKind::OrderedListItem { level, index } => {
            CompositeKind::OrderedListItemFollowUp { level, index }
        }
        other => other,
    }
}

fn slice_compounds<'s>(compounds: &[Compound<'s>], range: &Range<usize>) -> Vec<Compound<'s>> {
    let mut offset = 0;
    compounds
        .iter()
        .filter_map(|compound| {
            let (low, high) = (offset, offset + compound.src.len());
            offset = high;
            let (start, end) = (range.start.max(low), range.end.min(high));
            (start < end).then(|| compound.sub(start - low, end - low))
        })
        .collect()
}

pub fn break_lines(text: &str, first: usize, rest: usize) -> Vec<Range<usize>> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut fit = None;
    let breaks = break_opportunities(text);
    let mut next = 0;
    while let Some(&end) = breaks.get(next) {
        let limit = if lines.is_empty() { first } else { rest }.max(1);
        if end <= start || columns(text[start..end].trim_end()) <= limit {
            fit = (end > start).then_some(end);
            next += 1;
            continue;
        }
        let cut = fit.take().unwrap_or_else(|| hard_cut(text, start, limit));
        lines.push(start..start + text[start..cut].trim_end().len());
        start = cut;
    }
    if start < text.len() || lines.is_empty() {
        lines.push(start..text.trim_end().len().max(start));
    }
    lines
}

fn break_opportunities(text: &str) -> Vec<usize> {
    let mut visible = String::with_capacity(text.len());
    let mut origin = Vec::with_capacity(text.len());
    let mut previous_end = 0;
    for (index, ch) in text.char_indices().filter(|&(_, ch)| !is_marker(ch)) {
        origin.extend(std::iter::repeat_n(previous_end, ch.len_utf8()));
        visible.push(ch);
        previous_end = index + ch.len_utf8();
    }
    linebreaks(&visible)
        .map(|(index, _)| origin.get(index).copied().unwrap_or(text.len()))
        .collect()
}

fn hard_cut(text: &str, start: usize, limit: usize) -> usize {
    let mut used = 0;
    for (index, ch) in text[start..].char_indices() {
        used += ch.width().unwrap_or(0);
        if used > limit && index > 0 {
            return start + index;
        }
    }
    text.len()
}

fn columns(text: &str) -> usize {
    text.chars().map(|ch| ch.width().unwrap_or(0)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::{markdown_skin, Palette};

    #[test]
    fn wrap_cap_narrows_but_never_widens_the_terminal() {
        assert_eq!(capped_width(200, None), 200);
        assert_eq!(capped_width(200, Some(80)), 80);
        assert_eq!(capped_width(60, Some(80)), 60);
    }

    fn wrap(text: &str, width: usize) -> Vec<&str> {
        break_lines(text, width, width)
            .into_iter()
            .map(|range| &text[range])
            .collect()
    }

    fn strip_ansi(text: &str) -> String {
        let mut out = String::new();
        let mut chars = text.chars();
        while let Some(ch) = chars.next() {
            if ch == '\x1b' {
                chars.by_ref().find(|ch| ch.is_ascii_alphabetic());
            } else {
                out.push(ch);
            }
        }
        out
    }

    #[test]
    fn breaks_at_spaces_and_drops_trailing_space() {
        assert_eq!(
            wrap("the quick brown fox jumps", 10),
            ["the quick", "brown fox", "jumps"]
        );
    }

    #[test]
    fn short_text_stays_on_one_line() {
        assert_eq!(wrap("fits", 10), ["fits"]);
        assert_eq!(wrap("", 10), [""]);
    }

    #[test]
    fn first_line_can_be_narrower() {
        let text = "aaa bbb ccc";
        let lines: Vec<&str> = break_lines(text, 3, 7)
            .into_iter()
            .map(|range| &text[range])
            .collect();
        assert_eq!(lines, ["aaa", "bbb ccc"]);
    }

    #[test]
    fn cjk_counts_two_columns_and_breaks_between_ideographs() {
        assert_eq!(wrap("日本語のテキスト", 6), ["日本語", "のテキ", "スト"]);
    }

    #[test]
    fn emoji_counts_two_columns() {
        assert_eq!(wrap("📬 inbox 📬 mail", 8), ["📬 inbox", "📬 mail"]);
    }

    #[test]
    fn long_url_breaks_after_slash_not_mid_word() {
        assert_eq!(
            wrap("see https://example.com/some/long/path now", 20),
            ["see https://", "example.com/some/", "long/path now"]
        );
    }

    #[test]
    fn word_longer_than_width_is_cut_by_columns() {
        assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
    }

    #[test]
    fn wide_char_wider_than_width_still_progresses() {
        assert_eq!(wrap("日本", 1), ["日", "本"]);
    }

    #[test]
    fn no_break_before_closing_punctuation() {
        assert_eq!(wrap("aaa bbb!", 6), ["aaa", "bbb!"]);
    }

    #[test]
    fn markers_are_zero_width_and_move_with_the_next_word() {
        let text = "see \u{200B}docs\u{200C} \u{2061}#303\u{2062} now";
        assert_eq!(
            wrap(text, 4),
            ["see", "\u{200B}docs\u{200C}", "\u{2061}#303\u{2062}", "now"]
        );
    }

    #[test]
    fn markers_add_no_break_opportunity() {
        assert_eq!(
            wrap("(\u{200B}abc\u{200C})", 3),
            ["(\u{200B}ab", "c\u{200C})"]
        );
    }

    #[test]
    fn prose_wraps_list_items_with_follow_up_lines() {
        let skin = markdown_skin(&Palette::default());
        let rendered = strip_ansi(&render_prose(&skin, "* alpha beta gamma delta", 14));
        let lines: Vec<&str> = rendered.lines().map(str::trim_end).collect();
        assert_eq!(lines, ["• alpha beta", "  gamma delta"]);
    }

    #[test]
    fn prose_keeps_bold_style_on_both_sides_of_a_break() {
        let skin = markdown_skin(&Palette::default());
        let rendered = render_prose(&skin, "**aaaa bbbb**", 6);
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|line| line.contains("\x1b[1m")));
    }

    #[test]
    fn prose_keeps_tables_fitted_to_width() {
        let skin = markdown_skin(&Palette::default());
        let src = "|a|b|\n|-|-|\n|one two three four|five six seven eight|\n";
        let rendered = strip_ansi(&render_prose(&skin, src, 24));
        assert!(
            rendered.lines().all(|line| columns(line) <= 24),
            "{rendered}"
        );
        assert!(rendered.contains('│'));
    }

    #[test]
    fn prose_keeps_every_row_when_fitting_wraps_cells() {
        let skin = markdown_skin(&Palette::default());
        let src = "|a|b|\n|-|-|\n|one two three four|five six seven eight|\n|last|row|\n";
        let rendered = strip_ansi(&render_prose(&skin, src, 24));
        assert!(rendered.contains("last"), "{rendered}");
        assert!(rendered.contains("eight"), "{rendered}");
    }

    #[test]
    fn prose_wraps_inside_both_margins() {
        let mut skin = markdown_skin(&Palette::default());
        skin.paragraph.left_margin = 1;
        skin.paragraph.right_margin = 3;
        let rendered = strip_ansi(&render_prose(&skin, "aaaa bbbb cccc", 12));
        let lines: Vec<&str> = rendered.lines().map(str::trim).collect();
        assert_eq!(lines, ["aaaa", "bbbb", "cccc"]);
    }

    #[test]
    fn prose_never_wraps_code_lines() {
        let skin = markdown_skin(&Palette::default());
        let rendered = strip_ansi(&render_prose(&skin, "```\naaaa bbbb cccc\n```", 8));
        let lines: Vec<&str> = rendered.lines().map(str::trim).collect();
        assert_eq!(lines, ["aaaa bbbb cccc"]);
    }

    #[test]
    fn prose_slices_compounds_after_a_styled_one() {
        let skin = markdown_skin(&Palette::default());
        let rendered = strip_ansi(&render_prose(&skin, "aa **bb** cc dd", 6));
        let lines: Vec<&str> = rendered.lines().map(str::trim_end).collect();
        assert_eq!(lines, ["aa bb", "cc dd"]);
    }

    #[test]
    fn prose_hard_cut_at_a_style_boundary_adds_no_empty_styled_compound() {
        let skin = markdown_skin(&Palette::default());
        let rendered = render_prose(&skin, "aaaa**bb**", 4);
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(!lines[0].contains("\x1b[1m"), "{rendered:?}");
        assert!(lines[1].contains("\x1b[1m"), "{rendered:?}");
    }
}
