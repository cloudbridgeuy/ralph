use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Tag, TagEnd};
use syntect::parsing::SyntaxSet;

use crate::render::code::{highlight, CodeBlock};
use crate::render::escape;
use crate::render::md_link::{link_dest, safe_href};

const MAX_DEPTH: usize = 64;

struct PendingCode {
    lang: String,
    source: String,
}

struct Emitter<'s> {
    syntaxes: &'s SyntaxSet,
    pending_code: Option<PendingCode>,
    out: String,
    depth: usize,
    overflow: usize,
    image_depth: usize,
    in_head: bool,
    link_close: &'static str,
}

pub fn emit<'a>(events: impl Iterator<Item = Event<'a>>, syntaxes: &SyntaxSet) -> String {
    let mut emitter = Emitter::new(syntaxes);
    for event in events {
        emitter.event(event);
    }
    emitter.out
}

impl<'s> Emitter<'s> {
    fn new(syntaxes: &'s SyntaxSet) -> Self {
        Self {
            syntaxes,
            pending_code: None,
            out: String::new(),
            depth: 0,
            overflow: 0,
            image_depth: 0,
            in_head: false,
            link_close: "",
        }
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(end) => self.end(end),
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => self.text(&text),
            Event::Code(text) => self.code(&text),
            Event::SoftBreak => self.out.push('\n'),
            Event::HardBreak => self.out.push_str(self.break_text()),
            Event::Rule => self.block("<hr>"),
            Event::TaskListMarker(done) => self.task_marker(done),
            _ => {}
        }
    }

    fn break_text(&self) -> &'static str {
        if self.image_depth > 0 {
            " "
        } else {
            "<br>"
        }
    }

    fn text(&mut self, text: &str) {
        if let Some(code) = self.pending_code.as_mut() {
            code.source.push_str(text);
            return;
        }
        self.out.push_str(&escape(text).replace('\0', "\u{FFFD}"));
    }

    fn code(&mut self, text: &str) {
        if self.image_depth > 0 {
            self.text(text);
        } else {
            self.out.push_str("<code>");
            self.text(text);
            self.out.push_str("</code>");
        }
    }

    fn block(&mut self, html: &str) {
        if self.overflow == 0 && self.image_depth == 0 {
            self.out.push_str(html);
        }
    }

    fn task_marker(&mut self, done: bool) {
        let checked = if done { " checked" } else { "" };
        self.block(&format!("<input type=\"checkbox\" disabled{checked}> "));
    }

    fn start(&mut self, tag: Tag<'_>) {
        if self.depth >= MAX_DEPTH {
            self.overflow += 1;
            return;
        }
        self.depth += 1;
        match tag {
            Tag::Image { .. } => {
                self.image_depth += 1;
                if self.image_depth == 1 {
                    self.out.push_str("<span>");
                }
            }
            _ if self.image_depth > 0 => {}
            Tag::CodeBlock(kind) => self.pending_code = Some(pending_code(&kind)),
            Tag::Link {
                link_type,
                dest_url,
                ..
            } => self.open_link(&link_dest(link_type, &dest_url)),
            Tag::TableHead => {
                self.in_head = true;
                self.out.push_str("<table><thead><tr>");
            }
            other => self.out.push_str(&open_tag(&other, self.in_head)),
        }
    }

    fn open_link(&mut self, dest: &str) {
        match safe_href(dest) {
            Some(href) => {
                self.out.push_str(&format!(
                    "<a href=\"{}\" rel=\"noreferrer noopener\">",
                    escape(&href)
                ));
                self.link_close = "</a>";
            }
            None => {
                self.out.push_str("<span>");
                self.link_close = "</span>";
            }
        }
    }

    fn finish_code(&mut self) {
        if let Some(code) = self.pending_code.take() {
            let block = CodeBlock {
                lang: &code.lang,
                source: &code.source,
            };
            self.out.push_str(&highlight(self.syntaxes, &block));
        }
    }

    fn end(&mut self, end: TagEnd) {
        if self.overflow > 0 {
            self.overflow -= 1;
            self.out.push('\n');
            return;
        }
        self.depth = self.depth.saturating_sub(1);
        let in_head = self.in_head;
        match end {
            TagEnd::Image => {
                self.image_depth = self.image_depth.saturating_sub(1);
                if self.image_depth == 0 {
                    self.out.push_str("</span>");
                }
            }
            _ if self.image_depth > 0 => {}
            TagEnd::CodeBlock => self.finish_code(),
            TagEnd::Link => self.out.push_str(self.link_close),
            TagEnd::TableHead => {
                self.in_head = false;
                self.out.push_str("</tr></thead><tbody>");
            }
            TagEnd::Table => self.out.push_str("</tbody></table>"),
            other => self.out.push_str(close_tag(&other, in_head)),
        }
    }
}

fn pending_code(kind: &CodeBlockKind<'_>) -> PendingCode {
    let lang = match kind {
        CodeBlockKind::Fenced(info) => info.split_whitespace().next().unwrap_or_default(),
        CodeBlockKind::Indented => "",
    };
    PendingCode {
        lang: lang.to_string(),
        source: String::new(),
    }
}

fn heading(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn open_tag(tag: &Tag<'_>, in_head: bool) -> String {
    match tag {
        Tag::Paragraph | Tag::HtmlBlock => "<p>".into(),
        Tag::Heading { level, .. } => format!("<h{}>", heading(*level)),
        Tag::BlockQuote(_) => "<blockquote>".into(),
        Tag::List(Some(1)) => "<ol>".into(),
        Tag::List(Some(start)) => format!("<ol start=\"{start}\">"),
        Tag::List(None) => "<ul>".into(),
        Tag::Item => "<li>".into(),
        Tag::TableRow => "<tr>".into(),
        Tag::TableCell if in_head => "<th scope=\"col\">".into(),
        Tag::TableCell => "<td>".into(),
        Tag::Emphasis => "<em>".into(),
        Tag::Strong => "<strong>".into(),
        Tag::Strikethrough => "<del>".into(),
        _ => String::new(),
    }
}

fn close_tag(end: &TagEnd, in_head: bool) -> &'static str {
    match end {
        TagEnd::Paragraph | TagEnd::HtmlBlock => "</p>",
        TagEnd::Heading(HeadingLevel::H1) => "</h1>",
        TagEnd::Heading(HeadingLevel::H2) => "</h2>",
        TagEnd::Heading(HeadingLevel::H3) => "</h3>",
        TagEnd::Heading(HeadingLevel::H4) => "</h4>",
        TagEnd::Heading(HeadingLevel::H5) => "</h5>",
        TagEnd::Heading(HeadingLevel::H6) => "</h6>",
        TagEnd::BlockQuote(_) => "</blockquote>",
        TagEnd::List(true) => "</ol>",
        TagEnd::List(false) => "</ul>",
        TagEnd::Item => "</li>",
        TagEnd::TableRow => "</tr>",
        TagEnd::TableCell if in_head => "</th>",
        TagEnd::TableCell => "</td>",
        TagEnd::Emphasis => "</em>",
        TagEnd::Strong => "</strong>",
        TagEnd::Strikethrough => "</del>",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::{emit, MAX_DEPTH};
    use crate::render::prose::syntaxes;
    use pulldown_cmark::{Options, Parser};

    fn html(md: &str) -> String {
        let options =
            Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
        emit(Parser::new_ext(md, options), syntaxes())
    }

    #[test]
    fn paragraph_with_inline_marks() {
        assert_eq!(
            html("a **b** *c* ~~d~~ `e`"),
            "<p>a <strong>b</strong> <em>c</em> <del>d</del> <code>e</code></p>"
        );
    }

    #[test]
    fn raw_html_is_escaped_text() {
        assert_eq!(
            html("<b>x</b>\n\ny <i>z</i>"),
            "<p>&lt;b&gt;x&lt;/b&gt;</p><p>y &lt;i&gt;z&lt;/i&gt;</p>"
        );
    }

    #[test]
    fn safe_link_gets_rel() {
        assert_eq!(
            html("[t](https://a.b)"),
            "<p><a href=\"https://a.b\" rel=\"noreferrer noopener\">t</a></p>"
        );
    }

    #[test]
    fn spaces_in_a_link_destination_are_encoded() {
        assert_eq!(
            html("[t](<https://ex.com/a b>)"),
            "<p><a href=\"https://ex.com/a%20b\" rel=\"noreferrer noopener\">t</a></p>"
        );
    }

    #[test]
    fn email_autolink_gets_a_mailto_href() {
        assert_eq!(
            html("<a@b.c>"),
            "<p><a href=\"mailto:a@b.c\" rel=\"noreferrer noopener\">a@b.c</a></p>"
        );
    }

    #[test]
    fn unsafe_link_keeps_text_without_href() {
        assert_eq!(html("[t](javascript:x)"), "<p><span>t</span></p>");
    }

    #[test]
    fn entity_encoded_scheme_is_decoded_then_dropped() {
        assert_eq!(html("[t](&#106;avascript:x)"), "<p><span>t</span></p>");
    }

    #[test]
    fn image_becomes_alt_text_span() {
        assert_eq!(html("![a *b*](http://x/y.png)"), "<p><span>a b</span></p>");
    }

    #[test]
    fn table_has_head_and_body() {
        assert_eq!(
            html("|a|b|\n|-|-|\n|1|2|\n"),
            "<table><thead><tr><th scope=\"col\">a</th><th scope=\"col\">b</th></tr></thead><tbody><tr><td>1</td><td>2</td></tr></tbody></table>"
        );
    }

    #[test]
    fn ordered_list_keeps_a_start_other_than_one() {
        assert_eq!(html("1. a"), "<ol><li>a</li></ol>");
        assert_eq!(html("3. a"), "<ol start=\"3\"><li>a</li></ol>");
    }

    #[test]
    fn task_items_get_disabled_checkboxes() {
        assert_eq!(
            html("- [x] a\n- [ ] b"),
            "<ul><li><input type=\"checkbox\" disabled checked> a</li><li><input type=\"checkbox\" disabled> b</li></ul>"
        );
    }

    #[test]
    fn fenced_code_is_escaped() {
        assert_eq!(
            html("```\n<a>\n```"),
            "<r-code><pre><code>&lt;a&gt;\n</code></pre></r-code>"
        );
    }

    #[test]
    fn fenced_code_with_a_language_is_classed() {
        let out = html("```rust extra\nfn x() {}\n```");
        assert!(out.starts_with("<r-code data-lang=\"rust\"><pre><code>"));
        assert!(out.contains("class=\"t-"));
    }

    #[test]
    fn indented_code_has_no_language() {
        assert_eq!(
            html("    <a>\n"),
            "<r-code><pre><code>&lt;a&gt;\n</code></pre></r-code>"
        );
    }

    #[test]
    fn code_inside_deep_nesting_still_renders_escaped() {
        let md = "> ".repeat(MAX_DEPTH + 5) + "```\n<a>\n```";
        assert!(!html(&md).contains("<a>"));
    }

    #[test]
    fn nesting_beyond_the_cap_renders_as_text() {
        let md = "> ".repeat(MAX_DEPTH + 20) + "deep";
        let out = html(&md);
        assert_eq!(out.matches("<blockquote>").count(), MAX_DEPTH);
        assert_eq!(out.matches("</blockquote>").count(), MAX_DEPTH);
        assert!(out.contains("deep"));
    }

    #[test]
    fn nul_bytes_become_the_replacement_character() {
        assert_eq!(html("a\0b"), "<p>a\u{FFFD}b</p>");
    }
}
