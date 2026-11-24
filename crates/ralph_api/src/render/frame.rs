use crate::render::{escape, Shape};

pub fn frame(
    shape: Shape,
    title: Option<&str>,
    inner_html: &str,
    aside_html: Option<&str>,
) -> String {
    let (attrs, title) = title.map_or_else(Default::default, |text| {
        let text = escape(text);
        (
            format!(" role=\"group\" aria-label=\"{text}\""),
            format!("<div class=\"title\">{text}</div>"),
        )
    });
    let aside = aside_html
        .map(|html| format!("<div class=\"aside\">{html}</div>"))
        .unwrap_or_default();
    format!(
        "<r-box shape=\"{}\"{attrs}>{title}{aside}{inner_html}</r-box>",
        shape.as_str()
    )
}

#[cfg(test)]
mod tests {
    use super::frame;
    use crate::render::golden::{assert_golden, hostile_text, long_text};
    use crate::render::{escape, Shape};

    #[test]
    fn empty_box() {
        assert_golden("frame_empty", &frame(Shape::Square, None, "", None));
    }

    #[test]
    fn titled_box() {
        assert_golden(
            "frame_titled",
            &frame(Shape::Round, Some("Usage"), "<p>body</p>", None),
        );
    }

    #[test]
    fn hostile_title_is_escaped() {
        assert_golden(
            "frame_hostile_title",
            &frame(Shape::Heavy, Some(hostile_text()), "", None),
        );
    }

    #[test]
    fn very_long_title_and_body() {
        let long = long_text();
        assert_golden(
            "frame_very_long",
            &frame(Shape::Square, Some(&long), &format!("<p>{long}</p>"), None),
        );
    }

    #[test]
    fn aside_follows_the_title() {
        assert_golden(
            "frame_aside",
            &frame(Shape::Round, Some("Usage"), "<p>body</p>", Some("3 items")),
        );
    }

    #[test]
    fn aside_without_a_title_is_the_first_child() {
        assert_golden(
            "frame_aside_no_title",
            &frame(Shape::Square, None, "<p>body</p>", Some("3 items")),
        );
    }

    #[test]
    fn aside_html_is_trusted_and_the_caller_escapes() {
        let html = frame(Shape::Heavy, None, "", Some(&escape("<script>\"x\"'")));
        assert!(!html.contains("<script>"));
        assert_golden("frame_aside_hostile", &html);
    }
}
