use syntect::html::{ClassStyle, ClassedHTMLGenerator};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

use crate::render::escape;

const MAX_HIGHLIGHT_BYTES: usize = 64 * 1024;
const MAX_LANG_LEN: usize = 32;
const CLASS_STYLE: ClassStyle = ClassStyle::SpacedPrefixed { prefix: "t-" };

pub struct CodeBlock<'a> {
    pub lang: &'a str,
    pub source: &'a str,
}

pub fn highlight(syntaxes: &SyntaxSet, block: &CodeBlock<'_>) -> String {
    let lang = valid_lang(block.lang);
    let source = block.source.replace('\0', "\u{FFFD}");
    let body = classed(syntaxes, lang.as_deref(), &source).unwrap_or_else(|| escape(&source));
    let attr = lang
        .map(|lang| format!(" data-lang=\"{lang}\""))
        .unwrap_or_default();
    format!("<r-code{attr}><pre><code>{body}</code></pre></r-code>")
}

fn valid_lang(raw: &str) -> Option<String> {
    let lang = raw.to_ascii_lowercase();
    let ok = !lang.is_empty()
        && lang.len() <= MAX_LANG_LEN
        && lang.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'+' | b'#' | b'-')
        });
    ok.then_some(lang)
}

fn classed(syntaxes: &SyntaxSet, lang: Option<&str>, source: &str) -> Option<String> {
    if source.len() > MAX_HIGHLIGHT_BYTES {
        return None;
    }
    let syntax = syntaxes.find_syntax_by_token(lang?)?;
    let mut generator = ClassedHTMLGenerator::new_with_class_style(syntax, syntaxes, CLASS_STYLE);
    for line in LinesWithEndings::from(source) {
        generator
            .parse_html_for_line_which_includes_newline(line)
            .ok()?;
    }
    Some(generator.finalize())
}

#[cfg(test)]
mod tests {
    use super::{highlight, valid_lang, CodeBlock, MAX_HIGHLIGHT_BYTES};
    use crate::render::golden::{assert_golden, long_text};
    use crate::render::prose::syntaxes;

    fn render(lang: &str, source: &str) -> String {
        highlight(syntaxes(), &CodeBlock { lang, source })
    }

    #[test]
    fn rust_block() {
        assert_golden(
            "code_rust",
            &render("rust", "fn main() {\n    let s = \"a<b\"; // hi\n}\n"),
        );
    }

    #[test]
    fn unknown_language_is_plain_text() {
        assert_golden("code_unknown", &render("nosuchlang", "let x = 1 < 2;\n"));
    }

    #[test]
    fn hostile_language_is_dropped() {
        assert_golden("code_hostile_lang", &render("\"><script>", "<b>x</b>\n"));
    }

    #[test]
    fn very_long_block_is_plain_text() {
        let source = format!("// {}\n", long_text().repeat(MAX_HIGHLIGHT_BYTES / 100));
        assert!(source.len() > MAX_HIGHLIGHT_BYTES);
        let out = render("rust", &source);
        assert!(!out.contains("<span"));
        assert_golden("code_very_long", &out.replace(&source, "SOURCE"));
    }

    #[test]
    fn lang_is_validated() {
        assert_eq!(valid_lang("Rust"), Some("rust".into()));
        assert_eq!(valid_lang("c++"), Some("c++".into()));
        assert_eq!(valid_lang("c#"), Some("c#".into()));
        assert_eq!(valid_lang(""), None);
        assert_eq!(valid_lang("a b"), None);
        assert_eq!(valid_lang("\"><script>"), None);
        assert_eq!(valid_lang(&"a".repeat(33)), None);
        assert_eq!(valid_lang(&"a".repeat(32)), Some("a".repeat(32)));
    }

    #[test]
    fn no_language_has_no_data_lang() {
        assert_eq!(
            render("", "a\n"),
            "<r-code><pre><code>a\n</code></pre></r-code>"
        );
    }

    #[test]
    fn nul_bytes_become_the_replacement_character() {
        assert!(render("", "a\0b").contains("a\u{FFFD}b"));
    }
}
