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
fn private_use_markers_in_input_are_stripped() {
    let text = "a\u{E000}b\u{E005}c\u{E00A}d";
    assert_eq!(render(text, 80, &on(), true, false), "abcd\n");
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
