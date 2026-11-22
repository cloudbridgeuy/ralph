mod escape;

pub use escape::escape;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Ok,
    Bad,
    Warn,
    Info,
    Accent,
}

impl Tone {
    pub fn as_str(self) -> &'static str {
        match self {
            Tone::Ok => "ok",
            Tone::Bad => "bad",
            Tone::Warn => "warn",
            Tone::Info => "info",
            Tone::Accent => "accent",
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
        let names: Vec<&str> = [Tone::Ok, Tone::Bad, Tone::Warn, Tone::Info, Tone::Accent]
            .into_iter()
            .map(Tone::as_str)
            .collect();
        assert_eq!(names, ["ok", "bad", "warn", "info", "accent"]);
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
