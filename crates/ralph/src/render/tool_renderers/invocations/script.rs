use ralph_core::code_frame::Label;

use crate::render::code_frame::framed;
use crate::render::tool_renderers::context::{ansi, RenderContext};

pub fn render_script_invocation(
    ctx: &RenderContext,
    code: &str,
    allow_writes: bool,
    allow_spend: bool,
    timeout_secs: Option<u64>,
) -> String {
    let flags = [
        allow_writes.then(|| " [writes]".to_string()),
        allow_spend.then(|| " [spend]".to_string()),
        timeout_secs.map(|secs| format!(" [timeout {secs}s]")),
    ]
    .into_iter()
    .flatten()
    .collect::<String>();
    let code = code.trim_end_matches('\n');
    let body = match (code.is_empty(), ctx.terminal) {
        (true, _) => String::new(),
        (false, true) => {
            let highlighted = ctx
                .highlighter
                .highlight_foreground(code, Some("javascript"));
            framed(ctx, &Label::Language("javascript"), &highlighted)
        }
        (false, false) => format!("{code}\n"),
    };
    let header = match ctx.terminal {
        true => format!(
            "{}▶ mcptools execute{}{}{}{}\n",
            ctx.tools.name_fg(),
            ansi::RESET,
            ctx.tools.args_fg(),
            flags,
            ansi::RESET
        ),
        false => format!("> mcptools execute{flags}\n"),
    };
    format!("{header}{body}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ansi::truecolor_background;
    use crate::frame::strip_ansi;
    use crate::highlight::{Highlighter, ThemeConfig};
    use crate::render::Viewport;
    use crate::tool_theme::test_colors;

    #[test]
    fn header_names_the_tool_and_shows_only_set_flags() {
        let highlighter = Highlighter::new();
        let ctx = RenderContext::plain(&highlighter);
        let bare = render_script_invocation(&ctx, "", false, false, None);
        assert_eq!(bare, "> mcptools execute\n");
        let all = render_script_invocation(&ctx, "", true, true, Some(30));
        assert_eq!(all, "> mcptools execute [writes] [spend] [timeout 30s]\n");
    }

    #[test]
    fn each_code_line_is_one_output_line() {
        let highlighter = Highlighter::new();
        let ctx = RenderContext::plain(&highlighter);
        let out = render_script_invocation(
            &ctx,
            "const a = 1;\nreturn a;\n\nlog(a);",
            false,
            false,
            None,
        );
        assert_eq!(
            out,
            "> mcptools execute\nconst a = 1;\nreturn a;\n\nlog(a);\n"
        );
    }

    #[test]
    fn long_lines_are_not_cut() {
        let highlighter = Highlighter::new();
        let ctx = RenderContext::plain(&highlighter);
        let line = "x".repeat(500);
        assert!(render_script_invocation(&ctx, &line, false, false, None).contains(&line));
    }

    #[test]
    fn terminal_script_body_sits_in_the_shaded_code_frame() {
        let highlighter = Highlighter::with_config(ThemeConfig::default()).unwrap();
        let ctx = RenderContext::terminal(&highlighter)
            .with_viewport(Viewport::fixed(24, 24))
            .with_tools(test_colors());
        let out = render_script_invocation(&ctx, "const a = 1;\nreturn a;\n", true, false, None);
        assert!(out.starts_with("\x1b[94m▶ mcptools execute"));
        assert_eq!(
            strip_ansi(&out),
            format!(
                "▶ mcptools execute [writes]\n── javascript {}\n  \n  const a = 1;\n  return a;\n  \n",
                "─".repeat(10)
            )
        );
        let fill = truecolor_background(55, 60, 71);
        assert!(
            out.lines().skip(2).all(|row| row.starts_with(&fill)),
            "{out:?}"
        );
    }

    #[test]
    fn plain_context_has_no_escape_byte() {
        let highlighter = Highlighter::new();
        let ctx = RenderContext::plain(&highlighter);
        assert!(!render_script_invocation(&ctx, "return 1;", true, true, Some(5)).contains('\x1b'));
    }
}
