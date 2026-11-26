use ralph_core::code_frame::{frame_block, Columns, Label};

use super::RenderContext;

pub(crate) fn framed(ctx: &RenderContext<'_>, label: &Label<'_>, body: &str) -> String {
    let cols = Columns(u16::try_from(ctx.width).unwrap_or(u16::MAX));
    frame_block(label, body, cols)
}

#[cfg(test)]
mod tests {
    use crate::frame::strip_ansi;
    use crate::highlight::{Highlighter, ThemeConfig};
    use crate::markdown::MarkdownTheme;
    use crate::render::text::{render_code_block, render_markdown};
    use crate::render::{RenderContext, Viewport};

    fn highlighter() -> Highlighter {
        Highlighter::with_config(ThemeConfig::default()).unwrap()
    }

    fn terminal(highlighter: &Highlighter, cols: u16) -> RenderContext<'_> {
        let mut ctx = RenderContext::terminal(highlighter).with_viewport(Viewport::fixed(cols, 24));
        ctx.images = None;
        ctx
    }

    #[test]
    fn terminal_code_block_prints_no_fence() {
        let highlighter = highlighter();
        let out = render_code_block(&terminal(&highlighter, 20), "fn main() {}", Some("rust"));
        assert_eq!(
            strip_ansi(&out),
            "── rust ────────────\n  fn main() {}\n────────────────────\n"
        );
    }

    #[test]
    fn terminal_code_without_language_gets_a_bare_rule() {
        let highlighter = highlighter();
        let ctx = terminal(&highlighter, 6);
        for language in [None, Some("")] {
            let out = render_code_block(&ctx, "x", language);
            assert_eq!(strip_ansi(&out), "──────\n  x\n──────\n");
        }
    }

    #[test]
    fn terminal_code_paints_no_syntax_background() {
        let highlighter = highlighter();
        let out = render_code_block(&terminal(&highlighter, 20), "fn main() {}", Some("rust"));
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
            "── rust ──\n  a\n──────────\n\n── sh ────\n  b\n──────────\n\n"
        );
    }

    #[test]
    fn plain_code_block_keeps_raw_fences() {
        let highlighter = highlighter();
        let ctx = RenderContext::plain(&highlighter);
        let out = render_code_block(&ctx, "fn main() {}", Some("rust"));
        assert_eq!(out, "```rust\nfn main() {}\n```");
    }
}
