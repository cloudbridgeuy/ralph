use crate::render::tool_renderers::context::{ansi, RenderContext};
use crate::render::tool_renderers::types::ScriptResultParams;

fn section(ctx: &RenderContext, color: String, label: &str, lines: &[&str]) -> String {
    let heading = match ctx.terminal {
        true => format!("{color}{label}:{}", ansi::RESET),
        false => format!("{label}:"),
    };
    let body: String = lines.iter().map(|line| format!("  {line}\n")).collect();
    format!("{heading}\n{body}")
}

pub fn render_script_result(ctx: &RenderContext, params: &ScriptResultParams) -> String {
    let logs: Vec<&str> = params.logs.iter().flat_map(|log| log.lines()).collect();
    let mut out = String::new();
    if !logs.is_empty() {
        out.push_str(&section(ctx, ctx.tools.label_fg(), "logs", &logs));
    }
    if let Some(result) = params.result {
        let lines: Vec<&str> = result.lines().collect();
        out.push_str(&section(ctx, ctx.tools.label_fg(), "result", &lines));
    }
    if params.error.is_some() || params.is_error {
        let message = params.error.unwrap_or("(no message)");
        let lines: Vec<&str> = message.lines().collect();
        out.push_str(&section(ctx, ctx.tools.error_fg(), "error", &lines));
    }
    if let Some(trace_id) = params.trace_id {
        out.push_str(&match ctx.terminal {
            true => format!("{}traceId: {trace_id}{}\n", ansi::DIM, ansi::RESET),
            false => format!("traceId: {trace_id}\n"),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::Highlighter;
    use crate::tool_theme::test_colors;

    fn full() -> ScriptResultParams<'static> {
        ScriptResultParams {
            is_error: false,
            logs: &[],
            result: Some("42"),
            error: None,
            trace_id: Some("t-1"),
        }
    }

    #[test]
    fn plain_output_lists_each_section() {
        let highlighter = Highlighter::new();
        let ctx = RenderContext::plain(&highlighter);
        let logs = ["first".to_string(), "second".to_string()];
        let params = ScriptResultParams {
            logs: &logs,
            ..full()
        };
        assert_eq!(
            render_script_result(&ctx, &params),
            "logs:\n  first\n  second\nresult:\n  42\ntraceId: t-1\n"
        );
    }

    #[test]
    fn multiline_values_are_indented_line_by_line() {
        let highlighter = Highlighter::new();
        let ctx = RenderContext::plain(&highlighter);
        let params = ScriptResultParams {
            result: Some("a\nb"),
            trace_id: None,
            ..full()
        };
        assert_eq!(render_script_result(&ctx, &params), "result:\n  a\n  b\n");
    }

    #[test]
    fn error_section_uses_the_error_color() {
        let highlighter = Highlighter::new();
        let ctx = RenderContext::terminal(&highlighter).with_tools(test_colors());
        let params = ScriptResultParams {
            is_error: true,
            result: None,
            error: Some("boom"),
            trace_id: None,
            ..full()
        };
        let out = render_script_result(&ctx, &params);
        assert!(out.starts_with("\x1b[92merror:"));
        assert!(out.contains("boom"));
    }

    #[test]
    fn trace_id_is_one_dimmed_line() {
        let highlighter = Highlighter::new();
        let ctx = RenderContext::terminal(&highlighter).with_tools(test_colors());
        let params = ScriptResultParams {
            result: None,
            ..full()
        };
        let out = render_script_result(&ctx, &params);
        assert!(out.starts_with("\x1b[2mtraceId: t-1"));
        assert_eq!(out.lines().count(), 1);
    }

    #[test]
    fn plain_context_has_no_escape_byte() {
        let highlighter = Highlighter::new();
        let ctx = RenderContext::plain(&highlighter);
        let logs = ["x".to_string()];
        let params = ScriptResultParams {
            is_error: true,
            logs: &logs,
            error: Some("boom"),
            ..full()
        };
        assert!(!render_script_result(&ctx, &params).contains('\x1b'));
    }
}
