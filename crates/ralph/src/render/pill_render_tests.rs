use ralph_core::chunk::ParsedChunk;
use ralph_core::kitty::Placement;
use unicode_width::UnicodeWidthChar;

use super::link::mark_links;
use super::marker::{is_marker, paint_links};
use super::text::{enhance_tables, render_prose, render_text_block};
use super::wrap::wrap_width;
use super::{ImageSettings, RenderContext, Viewport};
use crate::highlight::{Highlighter, ThemeConfig};
use crate::markdown::MarkdownTheme;
use crate::render::pill_style::PillStyle;

const NB: char = '\u{00A0}';
const APC: &str = "\x1b_Gf=100,a=T;QQ==\x1b\\";

fn highlighter() -> Highlighter {
    Highlighter::with_config(ThemeConfig::default()).unwrap()
}

fn render(text: &str, cols: u16, theme: &MarkdownTheme, terminal: bool, images: bool) -> String {
    let highlighter = highlighter();
    let base = if terminal {
        RenderContext::terminal(&highlighter)
    } else {
        RenderContext::plain(&highlighter)
    };
    let mut ctx = base.with_viewport(Viewport::fixed(cols, 24));
    ctx.images = images.then_some(ImageSettings {
        placement: Placement::Direct,
        cell_px: None,
    });
    render_text_block(&ctx, &ParsedChunk::prose(text.to_string()), Some(theme))
}

fn mark_free(text: &str, cols: u16, theme: &MarkdownTheme) -> String {
    paint_links(
        &enhance_tables(&render_prose(
            &theme.skin,
            &mark_links(text),
            wrap_width(cols.into()),
        )),
        &theme.palette,
    )
}

fn visible(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else if !is_marker(ch) {
            out.push(ch);
        }
    }
    out
}

fn on() -> MarkdownTheme {
    MarkdownTheme::default().with_emphasis(true)
}

#[test]
fn flag_off_is_byte_identical_to_the_mark_free_pipeline() {
    let theme = MarkdownTheme::default();
    let text = "Build [[Done]] ✓ and → next\n├── a\n";
    for cols in [30, 80] {
        assert_eq!(
            render(text, cols, &theme, true, false),
            mark_free(text, cols, &theme)
        );
    }
}

#[test]
fn flag_on_in_a_terminal_paints_pills_with_non_breaking_padding() {
    let out = render("Build [[Done]] ok\n", 80, &on(), true, false);
    assert!(out.contains("\x1b[1m"), "{out:?}");
    assert!(out.contains("\x1b[42m"), "{out:?}");
    assert!(out.contains(&format!("{NB}Done{NB}")), "{out:?}");
    assert!(!out.contains("[["), "{out:?}");
}

#[test]
fn flag_on_without_a_terminal_changes_nothing() {
    let text = "Build [[Done]] ✓\n";
    assert_eq!(
        render(text, 80, &on(), false, false),
        render(text, 80, &MarkdownTheme::default(), false, false)
    );
}

#[test]
fn a_pill_never_breaks_across_lines() {
    let text = "aaaa bbbb [[Done 3 files]] cccc dddd eeee\n";
    for cols in 15..=40 {
        let lines: Vec<String> = visible(&render(text, cols, &on(), true, false))
            .lines()
            .map(str::to_string)
            .collect();
        let pill = format!("{NB}Done{NB}3{NB}files{NB}");
        assert_eq!(
            lines.iter().filter(|line| line.contains(&pill)).count(),
            1,
            "{cols}: {lines:?}"
        );
    }
}

#[test]
fn text_without_pills_wraps_the_same_with_the_flag_on() {
    let text = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu\n";
    for cols in 10..=40 {
        assert_eq!(
            render(text, cols, &on(), true, false),
            render(text, cols, &MarkdownTheme::default(), true, false),
            "{cols}"
        );
    }
}

#[test]
fn lines_with_a_pill_stay_within_the_width() {
    let text = "aaaa bbbb [[Done 3 files]] cccc dddd eeee ✓ ffff\n";
    for cols in 16..=40 {
        for line in visible(&render(text, cols, &on(), true, false)).lines() {
            let width: usize = line.chars().map(|ch| ch.width().unwrap_or(0)).sum();
            assert!(width <= usize::from(cols), "{cols}: {line:?}");
        }
    }
}

#[test]
fn the_image_path_applies_pills_to_each_text_piece() {
    let text = format!("[[Done]] a\n{APC}[[Next]] b\n");
    let out = render(&text, 80, &on(), true, true);
    assert!(out.contains(APC));
    assert_eq!(out.matches("\x1b[42m").count(), 1, "{out:?}");
    assert_eq!(out.matches("\x1b[104m").count(), 1, "{out:?}");
    assert!(!out.contains("[["), "{out:?}");
}

#[test]
fn bidi_controls_survive_the_styled_pipeline_with_emphasis_off() {
    let text = "shalom \u{200F}\u{2066}x\u{2069} \u{202A}y\u{202C} \u{061C}z";
    let theme = MarkdownTheme::default();
    assert_eq!(render(text, 80, &theme, true, false), format!("{text}\n"));
}

#[test]
fn bidi_controls_survive_the_styled_pipeline_with_emphasis_on() {
    let text = "shalom \u{200F}\u{2066}x\u{2069} \u{202A}y\u{202C} \u{061C}z";
    assert_eq!(render(text, 80, &on(), true, false), format!("{text}\n"));
}

#[test]
fn noncharacter_markers_in_input_are_stripped() {
    let text = "a\u{FDD0}b\u{FDD5}c\u{FDDA}d";
    assert_eq!(render(text, 80, &on(), true, false), "abcd\n");
}

#[test]
fn nerd_font_glyphs_survive_the_styled_pipeline_with_emphasis_off() {
    let text = "a\u{E000}b\u{E00A}c\u{E0B0}d";
    let theme = MarkdownTheme::default();
    assert_eq!(render(text, 80, &theme, true, false), format!("{text}\n"));
}

#[test]
fn pills_with_diffstat_or_memory_ref_detail_are_painted_whole() {
    for (text, shown) in [
        ("[[Done +3 tests]]", "Done +3 tests"),
        ("[[Failed -2 files]]", "Failed -2 files"),
        ("[[Done #303]]", "Done #303"),
    ] {
        let out = render(text, 80, &on(), true, false);
        let want = format!("{NB}{}{NB}", shown.replace(' ', &NB.to_string()));
        assert!(visible(&out).contains(&want), "{out:?}");
        assert!(out.contains("\x1b[1m"), "{out:?}");
        assert!(out.ends_with("\x1b[22m\x1b[39m\x1b[49m\n"), "{out:?}");
        assert_eq!(out.matches("\x1b[39m").count(), 1, "{out:?}");
    }
}

#[test]
fn backslashes_in_the_detail_survive_the_styled_pipeline() {
    for (text, want) in [
        ("[[Done a\\]]\n", "Done a\\"),
        ("[[Done a\\b]]\n", "Done a\\b"),
    ] {
        let out = render(text, 80, &on(), true, false);
        let pill = format!("{NB}{}{NB}", want.replace(' ', &NB.to_string()));
        assert!(visible(&out).contains(&pill), "{text:?}: {out:?}");
        assert!(!out.contains("[["), "{text:?}: {out:?}");
    }
}

fn round_on() -> MarkdownTheme {
    on().with_pill_style(PillStyle {
        round: true,
        icons: false,
    })
}

const CAP_L: char = '\u{E0B6}';
const CAP_R: char = '\u{E0B4}';

#[test]
fn round_pills_paint_caps_in_the_tone_colour_around_the_body() {
    let theme = round_on();
    let out = render("[[Done]]", 80, &theme, true, false);
    let tone = crate::ansi::fg(theme.palette.pill_ok);
    let want = format!(
        "{tone}{CAP_L}\x1b[1m\x1b[42m\x1b[97m{NB}Done{NB}\x1b[22m\x1b[39m\x1b[49m{tone}{CAP_R}\x1b[39m\n"
    );
    assert_eq!(out, want);
}

#[test]
fn square_pills_keep_the_exact_guz_1650_bytes() {
    let out = render("[[Done]]", 80, &on(), true, false);
    assert_eq!(
        out,
        format!("\x1b[1m\x1b[42m\x1b[97m{NB}Done{NB}\x1b[22m\x1b[39m\x1b[49m\n")
    );
}

#[test]
fn round_flag_without_emphasis_draws_nothing() {
    let theme = MarkdownTheme::default().with_pill_style(PillStyle {
        round: true,
        icons: false,
    });
    let out = render("[[Done]]", 80, &theme, true, false);
    assert_eq!(out, "[[Done]]\n");
}

#[test]
fn a_round_pill_never_breaks_and_counts_each_cap_as_one_column() {
    let text = "aaaa bbbb [[Done 3 files]] cccc dddd eeee\n";
    let inner = format!("{NB}Done{NB}3{NB}files{NB}");
    let whole = format!("{CAP_L}{inner}{CAP_R}");
    for cols in 17..=40 {
        let shown = visible(&render(text, cols, &round_on(), true, false));
        let lines: Vec<&str> = shown.lines().collect();
        assert_eq!(
            lines.iter().filter(|line| line.contains(&whole)).count(),
            1,
            "{cols}: {lines:?}"
        );
        for line in lines {
            let width: usize = line.chars().map(|ch| ch.width().unwrap_or(0)).sum();
            assert!(width <= usize::from(cols), "{cols}: {line:?}");
        }
    }
}

#[test]
fn a_round_pill_in_a_blockquote_adds_no_extra_background() {
    let theme = round_on();
    let out = render("> [[Done]] ok\n", 80, &theme, true, false);
    let tone = crate::ansi::fg(theme.palette.pill_ok);
    assert!(
        out.contains(&format!("{tone}{CAP_L}\x1b[1m\x1b[42m")),
        "{out:?}"
    );
    assert_eq!(out.matches("\x1b[42m").count(), 1, "{out:?}");
    assert_eq!(out.matches("\x1b[49m").count(), 1, "{out:?}");
}

fn icons_on(round: bool) -> MarkdownTheme {
    on().with_pill_style(PillStyle { round, icons: true })
}

const DONE_ICON: char = '\u{F00C}';

#[test]
fn square_icon_pills_paint_the_icon_inside_the_body() {
    let out = render("[[Done]]", 80, &icons_on(false), true, false);
    assert_eq!(
        out,
        format!("\x1b[1m\x1b[42m\x1b[97m{NB}{DONE_ICON}{NB}Done{NB}\x1b[22m\x1b[39m\x1b[49m\n")
    );
}

#[test]
fn round_icon_pills_paint_caps_around_the_icon_body() {
    let theme = icons_on(true);
    let out = render("[[Done]]", 80, &theme, true, false);
    let tone = crate::ansi::fg(theme.palette.pill_ok);
    let want = format!(
        "{tone}{CAP_L}\x1b[1m\x1b[42m\x1b[97m{NB}{DONE_ICON}{NB}Done{NB}\x1b[22m\x1b[39m\x1b[49m{tone}{CAP_R}\x1b[39m\n"
    );
    assert_eq!(out, want);
}

#[test]
fn icons_flag_without_emphasis_draws_nothing() {
    let theme = MarkdownTheme::default().with_pill_style(PillStyle {
        round: false,
        icons: true,
    });
    assert_eq!(render("[[Done]]", 80, &theme, true, false), "[[Done]]\n");
}

#[test]
fn an_icon_pill_never_breaks_and_counts_the_icon_as_one_column() {
    let text = "aaaa bbbb [[Done 3 files]] cccc dddd eeee\n";
    let whole = format!("{NB}{DONE_ICON}{NB}Done{NB}3{NB}files{NB}");
    for round in [false, true] {
        for cols in 17..=40 {
            let shown = visible(&render(text, cols, &icons_on(round), true, false));
            let lines: Vec<&str> = shown.lines().collect();
            assert_eq!(
                lines.iter().filter(|line| line.contains(&whole)).count(),
                1,
                "{round} {cols}: {lines:?}"
            );
            for line in lines {
                let width: usize = line.chars().map(|ch| ch.width().unwrap_or(0)).sum();
                assert!(width <= usize::from(cols), "{round} {cols}: {line:?}");
            }
        }
    }
}

const ALL_WORDS: &str =
    "[[Done]] [[Failed]] [[Warning]] [[Note]] [[Info]] [[Decision]] [[Next]] x\n";

fn full_style_without_emphasis() -> MarkdownTheme {
    MarkdownTheme::default().with_pill_style(PillStyle::resolve(true, true, false))
}

#[test]
fn colour_off_leaves_every_pill_word_raw_with_round_and_icons_requested() {
    let theme = full_style_without_emphasis();
    let plain_theme = MarkdownTheme::default();
    for cols in [30, 80] {
        for terminal in [true, false] {
            let out = render(ALL_WORDS, cols, &theme, terminal, false);
            assert_eq!(
                out,
                render(ALL_WORDS, cols, &plain_theme, terminal, false),
                "{cols} {terminal}"
            );
            assert!(
                out.contains("[[Done]]") && out.contains("[[Failed]]"),
                "{out:?}"
            );
        }
    }
}

#[test]
fn colour_off_with_round_and_icons_matches_the_mark_free_pipeline() {
    let theme = full_style_without_emphasis();
    for cols in [30, 80] {
        assert_eq!(
            render(ALL_WORDS, cols, &theme, true, false),
            mark_free(ALL_WORDS, cols, &theme)
        );
    }
}

#[test]
fn emphasis_without_a_terminal_ignores_round_and_icons() {
    let theme = icons_on(true);
    assert_eq!(
        render(ALL_WORDS, 80, &theme, false, false),
        render(ALL_WORDS, 80, &MarkdownTheme::default(), false, false)
    );
}

#[test]
fn colour_off_resolution_clears_round_and_icons() {
    let active = crate::render::pill::emphasis_active(true, false);
    assert!(!active);
    let style = PillStyle::resolve(true, true, active);
    assert!(!style.round && !style.icons);
    assert_eq!(style, PillStyle::default());
}

#[test]
fn square_pills_without_flags_keep_the_exact_bytes_for_every_tone() {
    let theme = on();
    for (text, bg, label) in [
        ("[[Done]]", "42", "Done"),
        ("[[Failed]]", "41", "Failed"),
        ("[[Warning]]", "43", "Warning"),
        ("[[Note]]", "104", "Note"),
        ("[[Info]]", "104", "Info"),
        ("[[Decision]]", "104", "Decision"),
        ("[[Next]]", "104", "Next"),
    ] {
        let out = render(text, 80, &theme, true, false);
        assert_eq!(
            out,
            format!("\x1b[1m\x1b[{bg}m\x1b[97m{NB}{label}{NB}\x1b[22m\x1b[39m\x1b[49m\n"),
            "{text}"
        );
        assert_eq!(
            out,
            render(
                text,
                80,
                &on().with_pill_style(PillStyle::default()),
                true,
                false
            ),
            "{text}"
        );
    }
}

#[test]
fn nerd_font_glyphs_outside_pills_survive_with_round_and_icons_on() {
    let text = "a\u{E000}b\u{E00A}c\u{E0B0}d\u{E0B6}e\u{E0B4}f\u{F00C}g";
    for round in [false, true] {
        let out = render(
            &format!("{text} [[Done]]"),
            80,
            &icons_on(round),
            true,
            false,
        );
        let shown = visible(&out);
        assert!(shown.starts_with(&format!("{text} ")), "{round}: {out:?}");
        assert_eq!(out.matches("\x1b[42m").count(), 1, "{round}: {out:?}");
    }
    let alone = render(text, 80, &icons_on(true), true, false);
    assert_eq!(alone, format!("{text}\n"));
}

const BODY: &str = "\x1b[1m\x1b[42m\x1b[97m";
const WIDE: &str = "[[Done 3 files changed in total]]";

#[test]
fn wrapped_pill_lines_reopen_the_body_style_for_every_shape() {
    for (round, icons) in [(false, false), (false, true), (true, false), (true, true)] {
        let theme = on().with_pill_style(PillStyle { round, icons });
        for cols in 8..=14 {
            let out = render(WIDE, cols, &theme, true, false);
            let lines: Vec<&str> = out.lines().collect();
            assert!(lines.len() >= 3, "{round} {icons} {cols}: {lines:?}");
            let last = lines.len() - 1;
            for (index, line) in lines.iter().enumerate().skip(1) {
                let reopened = line.starts_with(BODY)
                    || (index == last && line.starts_with(&crate::ansi::fg(theme.palette.pill_ok)));
                assert!(reopened, "{round} {icons} {cols} {index}: {lines:?}");
            }
        }
    }
}

#[test]
fn wrapped_pill_ends_reset_with_no_leaked_background() {
    for (round, icons) in [(false, false), (false, true), (true, false), (true, true)] {
        let theme = on().with_pill_style(PillStyle { round, icons });
        let tone = crate::ansi::fg(theme.palette.pill_ok);
        let tail = if round {
            format!("{CAP_R}\x1b[39m\n")
        } else {
            format!("{NB}\x1b[22m\x1b[39m\x1b[49m\n")
        };
        for cols in 8..=14 {
            let out = render(WIDE, cols, &theme, true, false);
            assert!(out.ends_with(&tail), "{round} {icons} {cols}: {out:?}");
            assert!(out.contains("\x1b[49m"), "{round} {icons} {cols}: {out:?}");
            if round {
                let last = out.lines().last().unwrap_or_default();
                assert!(
                    last.starts_with(&tone) || last.starts_with(BODY),
                    "{round} {icons} {cols}: {out:?}"
                );
            }
        }
    }
}

#[test]
fn an_exact_fit_wrap_paints_the_left_cap_in_the_tone_colour() {
    let theme = round_on();
    let tone = crate::ansi::fg(theme.palette.pill_ok);
    let out = render("foo([[Done]])", 4, &theme, true, false);
    let line = out
        .lines()
        .find(|line| line.contains(CAP_L))
        .unwrap_or_default();
    assert!(line.starts_with(&tone), "{out:?}");
}

#[test]
fn a_right_cap_wrapped_to_line_two_is_painted_in_the_tone_after_the_body_closes() {
    let theme = round_on();
    let tone = crate::ansi::fg(theme.palette.pill_ok);
    let out = render("foo([[Done]])", 11, &theme, true, false);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2, "{out:?}");
    assert!(
        lines[0].starts_with(&format!("foo({tone}{CAP_L}{BODY}")),
        "{out:?}"
    );
    assert!(
        lines[0].contains(&format!("Done{NB}\x1b[22m\x1b[39m\x1b[49m")),
        "{out:?}"
    );
    assert!(lines[1].starts_with(&format!("{tone}{CAP_R}")), "{out:?}");
    assert!(lines[1].ends_with("\x1b[39m)"), "{out:?}");
}

#[test]
fn an_exact_fit_wrap_keeps_the_closing_padding_and_resets_the_style_on_line_one() {
    let out = render("foo([[Done]])", 11, &round_on(), true, false);
    let first = out.lines().next().unwrap_or_default();
    assert!(
        first.contains(&format!("{NB}Done{NB}\x1b[22m\x1b[39m\x1b[49m")),
        "{out:?}"
    );
}
