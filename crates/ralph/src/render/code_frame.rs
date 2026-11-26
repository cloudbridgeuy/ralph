use ralph_core::code_frame::{frame_block, Columns, Label, MARGIN};

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

    fn unfilled_characters(out: &str, fill: &str) -> String {
        let mut loose = String::new();
        for row in out.lines().skip(1) {
            let mut filled = false;
            let mut rest = row;
            while let Some(first) = rest.chars().next() {
                if let Some(after) = rest.strip_prefix(fill) {
                    filled = true;
                    rest = after;
                } else if first == '\x1b' {
                    let end = rest.find(|c: char| c.is_ascii_alphabetic()).unwrap() + 1;
                    filled &= &rest[..end] != "\x1b[0m";
                    rest = &rest[end..];
                } else {
                    if !filled {
                        loose.push(first);
                    }
                    rest = &rest[first.len_utf8()..];
                }
            }
        }
        loose
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
            assert_eq!(unfilled_characters(&out, &fill), "", "{out:?}");
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
        assert_eq!(unfilled_characters(&out, &fill), "", "{out:?}");
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
}
