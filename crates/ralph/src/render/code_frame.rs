use ralph_core::chunk::ChunkType;
use ralph_core::code_frame::{frame_block, BlockKind, Columns, Label, MARGIN};

use super::RenderContext;
use crate::diff_highlight::highlight_with_basic_colors;

pub(crate) fn framed(ctx: &RenderContext<'_>, label: &Label<'_>, body: &str) -> String {
    let cols = Columns(u16::try_from(ctx.width).unwrap_or(u16::MAX));
    frame_block(label, body, ctx.highlighter.surface(), cols)
}

pub(crate) fn framed_diff(ctx: &RenderContext<'_>, label: &Label<'_>, diff: &str) -> String {
    let coloured = highlight_with_basic_colors(diff, &ctx.diff);
    framed(ctx, label, without_line_end(&coloured))
}

pub(crate) fn block_kind(ctx: &RenderContext<'_>, chunk: &ChunkType) -> BlockKind {
    match chunk {
        ChunkType::Code { .. } | ChunkType::Diff if ctx.terminal => BlockKind::Frame,
        _ => BlockKind::Flow,
    }
}

pub(crate) fn body_width(ctx: &RenderContext<'_>) -> usize {
    if ctx.terminal {
        ctx.width.saturating_sub(MARGIN.len())
    } else {
        ctx.width
    }
}

pub(crate) fn without_line_end(text: &str) -> &str {
    text.strip_suffix('\n').unwrap_or(text)
}

#[cfg(test)]
mod tests {
    use crate::ansi::truecolor_background;
    use crate::frame::strip_ansi;
    use crate::highlight::{Highlighter, ThemeConfig};
    use crate::markdown::MarkdownTheme;
    use crate::render::text::{render_code_block, render_diff_block, render_markdown};
    use crate::render::{render_bash_invocation, RenderContext, Viewport};
    use crate::stream_processor::StreamProcessor;
    use serde_json::json;

    fn highlighter() -> Highlighter {
        Highlighter::with_config(ThemeConfig::default()).unwrap()
    }

    fn terminal(highlighter: &Highlighter, cols: u16) -> RenderContext<'_> {
        let mut ctx = RenderContext::terminal(highlighter).with_viewport(Viewport::fixed(cols, 24));
        ctx.images = None;
        ctx
    }

    fn open_highlighter() -> Highlighter {
        Highlighter::with_config(ThemeConfig::default().with_no_background(true)).unwrap()
    }

    fn background_after(codes: &str, current: Option<String>) -> Option<String> {
        let mut codes = codes.split(';');
        let mut background = current;
        while let Some(code) = codes.next() {
            match code.parse::<u16>().unwrap_or(0) {
                0 | 49 => background = None,
                code @ (40..=47 | 100..=107) => background = Some(code.to_string()),
                code @ (38 | 48) => {
                    let mode = codes.next().unwrap_or_default();
                    let take = if mode == "2" { 3 } else { 1 };
                    let values: Vec<&str> = codes.by_ref().take(take).collect();
                    if code == 48 {
                        background = Some(format!("\x1b[48;{mode};{}m", values.join(";")));
                    }
                }
                _ => {}
            }
        }
        background
    }

    fn unfilled_characters(rows: &str, fill: &str) -> String {
        let mut loose = String::new();
        let mut background = None;
        let mut rest = rows;
        while let Some(first) = rest.chars().next() {
            if let Some(sequence) = rest.strip_prefix("\x1b[") {
                let end = sequence.find(|c: char| c.is_ascii_alphabetic()).unwrap();
                if sequence[end..].starts_with('m') {
                    background = background_after(&sequence[..end], background);
                }
                rest = &sequence[end + 1..];
            } else {
                if first != '\n' && background.as_deref() != Some(fill) {
                    loose.push(first);
                }
                rest = &rest[first.len_utf8()..];
            }
        }
        loose
    }

    fn below_the_rule(out: &str) -> &str {
        out.split_once('\n').unwrap().1
    }

    #[test]
    fn terminal_code_block_prints_no_fence() {
        let highlighter = highlighter();
        let out = render_code_block(&terminal(&highlighter, 20), "fn main() {}", Some("rust"));
        assert_eq!(
            strip_ansi(&out),
            "── rust ────────────\n  \n  fn main() {}\n  \n"
        );
    }

    #[test]
    fn every_character_of_a_shaded_block_sits_on_the_fill() {
        let highlighter = highlighter();
        let code = "fn main() {\n    let a = \"b\"; // c\n}";
        for language in [Some("rust"), None] {
            let out = render_code_block(&terminal(&highlighter, 40), code, language);
            let fill = truecolor_background(55, 60, 71);
            assert!(out.contains(&fill), "{out:?}");
            assert_eq!(
                unfilled_characters(below_the_rule(&out), &fill),
                "",
                "{out:?}"
            );
        }
    }

    #[test]
    fn disabled_background_keeps_the_open_frame() {
        let highlighter = open_highlighter();
        let out = render_code_block(&terminal(&highlighter, 10), "a", Some("sh"));
        assert_eq!(strip_ansi(&out), "── sh ────\n  a\n──────────\n");
        assert!(!out.contains("\x1b[48;"), "{out:?}");
    }

    #[test]
    fn terminal_code_without_language_gets_a_bare_rule() {
        let highlighter = highlighter();
        let ctx = terminal(&highlighter, 6);
        for language in [None, Some("")] {
            let out = render_code_block(&ctx, "x", language);
            assert_eq!(strip_ansi(&out), "──────\n  \n  x\n  \n");
        }
    }

    #[test]
    fn foreground_highlight_has_no_background() {
        let highlighter = highlighter();
        let out = highlighter.highlight_foreground("fn main() {}", Some("rust"));
        assert!(out.contains("\x1b[38;2;"), "{out:?}");
        assert!(!out.contains("\x1b[48;2;"), "{out:?}");
    }

    #[test]
    fn back_to_back_blocks_have_one_blank_row() {
        let highlighter = highlighter();
        let text = "```rust\na\n```\n\n```sh\nb\n```\n";
        let out = render_markdown(&terminal(&highlighter, 10), text, &MarkdownTheme::default());
        assert_eq!(
            strip_ansi(&out),
            "── rust ──\n  \n  a\n  \n\n── sh ────\n  \n  b\n  \n\n"
        );
    }

    #[test]
    fn terminal_diff_has_no_fence_or_extra_row() {
        let highlighter = highlighter();
        let out = render_diff_block(&terminal(&highlighter, 12), "+a\n-b");
        assert_eq!(strip_ansi(&out), "── diff ────\n  \n  +a\n  -b\n  \n");
    }

    #[test]
    fn every_diff_character_sits_on_the_fill() {
        let highlighter = highlighter();
        let out = render_diff_block(&terminal(&highlighter, 30), "@@ -1 +1 @@\n-a\n+b\n c");
        let fill = truecolor_background(55, 60, 71);
        assert_eq!(
            unfilled_characters(below_the_rule(&out), &fill),
            "",
            "{out:?}"
        );
    }

    #[test]
    fn multiline_bash_call_is_framed() {
        let highlighter = highlighter();
        let out = render_bash_invocation(&terminal(&highlighter, 20), "a\nb\n");
        assert_eq!(
            strip_ansi(&out),
            "▶ Bash\n── sh ──────────────\n  \n  a\n  b\n  \n"
        );
    }

    #[test]
    fn single_line_terminal_bash_has_no_frame() {
        let highlighter = highlighter();
        let out = render_bash_invocation(&terminal(&highlighter, 20), "pwd");
        assert_eq!(strip_ansi(&out), "▶ Bash\n  pwd\n");
    }

    #[test]
    fn real_highlighter_output_stays_on_the_fill() {
        let highlighter = highlighter();
        let ctx = terminal(&highlighter, 60);
        let fill = truecolor_background(55, 60, 71);
        let script = "for f in *.rs; do\n\techo \"$f\" | wc -l # count\ndone";
        let patch = "diff --git a b\n--- a\n+++ b\n@@ -1 +1 @@\n-old\n+new\n same";
        for out in [
            render_code_block(&ctx, "cargo test", Some("sh")),
            render_code_block(&ctx, script, Some("bash")),
            render_diff_block(&ctx, patch),
        ] {
            assert!(out.contains("\x1b[3"), "{out:?}");
            assert_eq!(
                unfilled_characters(below_the_rule(&out), &fill),
                "",
                "{out:?}"
            );
        }
    }

    #[test]
    fn single_line_terminal_bash_paints_no_background() {
        let highlighter = highlighter();
        let out = render_bash_invocation(&terminal(&highlighter, 40), "cargo test");
        assert!(out.contains("\x1b[38;2;"), "{out:?}");
        assert!(!out.contains("\x1b[48;"), "{out:?}");
    }

    #[test]
    fn reprint_uses_current_columns() {
        let highlighter = highlighter();
        let mut ctx = terminal(&highlighter, 100);
        ctx.viewport = Viewport::fixed(60, 24);
        let out = render_markdown(&ctx, "```rust\na\n```\n", &MarkdownTheme::default());
        let rule = strip_ansi(out.lines().next().unwrap());
        assert_eq!(rule.chars().count(), 60, "{rule:?}");
    }

    #[test]
    fn shell_and_mermaid_width_excludes_the_margin() {
        let highlighter = highlighter();
        let mut ctx = terminal(&highlighter, 12);
        ctx.mermaid = |_, width| Some(width.to_string());
        let diagram = render_code_block(&ctx, "graph TD", Some("mermaid"));
        assert!(strip_ansi(&diagram).contains("\n  10\n"), "{diagram:?}");
        for out in [
            render_code_block(&ctx, "aaaa | bbbb", Some("sh")),
            render_bash_invocation(&ctx, "aaaa | bbbb"),
        ] {
            let plain = strip_ansi(&out);
            assert!(!plain.contains("aaaa | bbbb"), "{plain:?}");
        }
    }

    #[test]
    fn plain_shell_width_is_unchanged() {
        let highlighter = highlighter();
        let mut ctx = RenderContext::plain(&highlighter);
        ctx.width = 12;
        let out = render_code_block(&ctx, "aaaa | bbbb", Some("sh"));
        assert_eq!(out, "```sh\naaaa | bbbb\n```");
    }

    #[test]
    fn plain_code_block_keeps_raw_fences() {
        let highlighter = highlighter();
        let ctx = RenderContext::plain(&highlighter);
        let out = render_code_block(&ctx, "fn main() {}", Some("rust"));
        assert_eq!(out, "```rust\nfn main() {}\n```");
    }

    fn blank_rows_above(out: &str, rule: &str) -> usize {
        let plain = strip_ansi(out);
        let (above, _) = plain.split_once(rule).unwrap();
        above
            .lines()
            .rev()
            .take_while(|row| row.trim().is_empty())
            .count()
    }

    fn live(highlight: bool, events: &[serde_json::Value]) -> String {
        let mut processor = StreamProcessor::with_highlighting(highlight);
        let mut shown: String = events
            .iter()
            .filter_map(|event| processor.process_line(&event.to_string()))
            .collect();
        shown.push_str(&processor.finish_rendered().1);
        shown
    }

    fn said(text: &str) -> serde_json::Value {
        json!({"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":text}]}})
    }

    const TEXT_THEN_FRAME: [&str; 5] = [
        "# Title\n\n```rust\na\n```\n",
        "- item\n\n  ```rust\n  a\n  ```\n",
        "1. step\n```rust\na\n```\n",
        "Words.\n\n```rust\na\n```\n",
        "Words:\n```rust\na\n```\n",
    ];

    #[test]
    fn markdown_has_one_blank_row_above_a_frame() {
        let highlighter = highlighter();
        let ctx = terminal(&highlighter, 40);
        for text in TEXT_THEN_FRAME {
            let out = render_markdown(&ctx, text, &MarkdownTheme::default());
            assert_eq!(blank_rows_above(&out, "── rust "), 1, "{text:?} {out:?}");
        }
    }

    #[test]
    fn a_live_turn_has_one_blank_row_above_a_frame() {
        for text in TEXT_THEN_FRAME {
            let out = live(true, &[said(text)]);
            assert_eq!(blank_rows_above(&out, "── rust "), 1, "{text:?} {out:?}");
        }
    }

    #[test]
    fn a_frame_in_a_later_event_keeps_one_blank_row_above() {
        let out = live(true, &[said("Words:\n"), said("```rust\na\n```\n")]);
        assert_eq!(blank_rows_above(&out, "── rust "), 1, "{out:?}");
    }

    #[test]
    fn a_frame_that_opens_the_output_has_no_row_above() {
        let highlighter = highlighter();
        let text = "```rust\na\n```\n";
        for out in [
            render_markdown(&terminal(&highlighter, 40), text, &MarkdownTheme::default()),
            live(true, &[said(text)]),
        ] {
            assert!(strip_ansi(&out).starts_with("── rust "), "{out:?}");
        }
    }

    #[test]
    fn piped_output_gets_no_extra_row() {
        let out = live(false, &[said("Words:\n```rust\na\n```\n")]);
        assert_eq!(out, "Words:```rust\na\n```\n");
    }
}
