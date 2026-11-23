use std::sync::OnceLock;

use pulldown_cmark::{Options, Parser};
use syntect::parsing::SyntaxSet;

use crate::render::md_event::emit;

pub(crate) fn syntaxes() -> &'static SyntaxSet {
    static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
    SYNTAXES.get_or_init(two_face::syntax::extra_newlines)
}

pub fn render_markdown(markdown: &str) -> String {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    format!(
        "<r-prose>{}</r-prose>",
        emit(Parser::new_ext(markdown, options), syntaxes())
    )
}

#[cfg(test)]
mod tests {
    use super::render_markdown;
    use crate::render::golden::{assert_golden, long_text};

    #[test]
    fn plain_markdown() {
        let md = "# Title\n\nSome **bold**, *em*, ~~gone~~ and `code`.\n\n[site](https://example.com) and [mail](mailto:a@b.c) and [frag](#top)\n\n```\nlet x = 1 < 2;\n```\n\n> quote\n\n---\n";
        assert_golden("prose_plain", &render_markdown(md));
    }

    #[test]
    fn table_markdown() {
        let md = "| a | b |\n|---|:-:|\n| 1 | 2 |\n| <x> | 4 |\n";
        assert_golden("prose_table", &render_markdown(md));
    }

    #[test]
    fn list_markdown() {
        let md = "- one\n- two\n  - nested\n\n3. three\n4. four\n\n- [x] done\n- [ ] todo\n";
        assert_golden("prose_list", &render_markdown(md));
    }

    #[test]
    fn empty_markdown() {
        assert_golden("prose_empty", &render_markdown(""));
    }

    #[test]
    fn very_long_markdown() {
        assert_golden("prose_very_long", &render_markdown(&long_text()));
    }

    #[test]
    fn hostile_markdown() {
        let md = "<script>alert(1)</script>\n\n<img src=x onerror=alert(1)>\n\n[js](javascript:alert(1)) [data](data:text/html,x) [ent](&#106;avascript:alert(1)) [ws](java\tscript:alert(1)) ![pic](http://x/y.png \"t\")\n\n<div><b>unclosed\n\ninline <span onclick=x> text\n\nnul \u{0} byte\n";
        let out = render_markdown(md);
        assert_golden("prose_hostile", &out);
    }

    #[test]
    fn hostile_output_has_no_live_markup_from_input() {
        let out = render_markdown("x <script>x</script> [a](javascript:x) ![i](http://h/p.png)");
        assert!(!out.contains("<script"));
        assert!(!out.contains("javascript:"));
        assert!(!out.contains("<img"));
    }

    #[test]
    fn deep_block_quotes_are_capped() {
        let md = "> ".repeat(10_000) + "deep";
        let out = render_markdown(&md);
        assert_eq!(out.matches("<blockquote>").count(), 64);
        assert!(out.contains("deep"));
    }
}
