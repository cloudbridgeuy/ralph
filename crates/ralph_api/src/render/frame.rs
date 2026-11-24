use crate::render::{escape, Shape};

pub fn frame(shape: Shape, title: Option<&str>, inner_html: &str) -> String {
    let (attrs, title) = title.map_or_else(Default::default, |text| {
        let text = escape(text);
        (
            format!(" role=\"group\" aria-label=\"{text}\""),
            format!("<div class=\"title\">{text}</div>"),
        )
    });
    format!(
        "<r-box shape=\"{}\"{attrs}>{title}{inner_html}</r-box>",
        shape.as_str()
    )
}

#[cfg(test)]
mod tests {
    use super::frame;
    use crate::render::golden::{assert_golden, hostile_text, long_text};
    use crate::render::Shape;

    #[test]
    fn empty_box() {
        assert_golden("frame_empty", &frame(Shape::Square, None, ""));
    }

    #[test]
    fn titled_box() {
        assert_golden(
            "frame_titled",
            &frame(Shape::Round, Some("Usage"), "<p>body</p>"),
        );
    }

    #[test]
    fn hostile_title_is_escaped() {
        assert_golden(
            "frame_hostile_title",
            &frame(Shape::Heavy, Some(hostile_text()), ""),
        );
    }

    #[test]
    fn very_long_title_and_body() {
        let long = long_text();
        assert_golden(
            "frame_very_long",
            &frame(Shape::Square, Some(&long), &format!("<p>{long}</p>")),
        );
    }
}
