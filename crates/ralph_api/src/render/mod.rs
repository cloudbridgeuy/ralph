mod code;
mod empty;
mod error;
mod escape;
mod frame;
#[cfg(test)]
pub(crate) mod golden;
mod kv;
mod md_event;
mod md_link;
mod pill;
mod prose;
mod rule;
mod spark;
mod spinner;
mod status;
mod table;
mod tree;

pub use empty::empty;
pub use error::error;
pub use escape::escape;
pub use frame::frame;
pub use kv::kv;
pub use pill::{pill, pill_with, Icon, PillShape, PillVariant};
pub use prose::render_markdown;
pub use rule::rule;
pub use spark::spark_svg;
pub use spinner::spinner;
pub use status::status;
pub use table::table;
pub use tree::{tree, Cells, TreeNode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Ok,
    Bad,
    Warn,
    Info,
    Accent,
    Neutral,
}

impl Tone {
    pub fn as_str(self) -> &'static str {
        match self {
            Tone::Ok => "ok",
            Tone::Bad => "bad",
            Tone::Warn => "warn",
            Tone::Info => "info",
            Tone::Accent => "accent",
            Tone::Neutral => "neutral",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tone::Ok => "OK",
            Tone::Bad => "Error",
            Tone::Warn => "Warning",
            Tone::Info => "Info",
            Tone::Accent => "Note",
            Tone::Neutral => "Neutral",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Square,
    Round,
    Heavy,
}

impl Shape {
    pub fn as_str(self) -> &'static str {
        match self {
            Shape::Square => "square",
            Shape::Round => "round",
            Shape::Heavy => "heavy",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Shape, Tone};

    #[test]
    fn tone_names_are_the_closed_set() {
        let names: Vec<&str> = [
            Tone::Ok,
            Tone::Bad,
            Tone::Warn,
            Tone::Info,
            Tone::Accent,
            Tone::Neutral,
        ]
        .into_iter()
        .map(Tone::as_str)
        .collect();
        assert_eq!(names, ["ok", "bad", "warn", "info", "accent", "neutral"]);
    }

    #[test]
    fn tone_labels_are_words() {
        let labels: Vec<&str> = [
            Tone::Ok,
            Tone::Bad,
            Tone::Warn,
            Tone::Info,
            Tone::Accent,
            Tone::Neutral,
        ]
        .into_iter()
        .map(Tone::label)
        .collect();
        assert_eq!(
            labels,
            ["OK", "Error", "Warning", "Info", "Note", "Neutral"]
        );
    }

    #[test]
    fn shape_names_are_the_closed_set() {
        let names: Vec<&str> = [Shape::Square, Shape::Round, Shape::Heavy]
            .into_iter()
            .map(Shape::as_str)
            .collect();
        assert_eq!(names, ["square", "round", "heavy"]);
    }
}
