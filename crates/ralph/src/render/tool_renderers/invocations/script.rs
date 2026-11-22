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
        (false, true) => format!("{}\n", ctx.highlighter.highlight(code, Some("javascript"))),
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
    use crate::highlight::Highlighter;
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
    fn terminal_context_highlights_and_keeps_line_count() {
        let highlighter = Highlighter::new();
        let ctx = RenderContext::terminal(&highlighter).with_tools(test_colors());
        let out = render_script_invocation(&ctx, "const a = 1;\nreturn a;\n", true, false, None);
        assert!(out.starts_with("\x1b[94m▶ mcptools execute"));
        assert!(out.contains("[writes]"));
        assert!(out.contains('\x1b'));
        assert_eq!(out.lines().count(), 3);
    }

    #[test]
    fn plain_context_has_no_escape_byte() {
        let highlighter = Highlighter::new();
        let ctx = RenderContext::plain(&highlighter);
        assert!(!render_script_invocation(&ctx, "return 1;", true, true, Some(5)).contains('\x1b'));
    }
}
