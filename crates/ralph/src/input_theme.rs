use crate::markdown::parse_color;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use termimad::crossterm::style::Color;

const SUGGESTION_KEY: &str = "suggestion";
const SUGGESTION_DEFAULT: Color = Color::Cyan;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InputColors {
    pub suggestion: Color,
}

impl Default for InputColors {
    fn default() -> Self {
        Self {
            suggestion: SUGGESTION_DEFAULT,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct InputTheme {
    #[serde(flatten)]
    pub colors: BTreeMap<String, String>,
}

impl InputTheme {
    pub fn resolve(&self) -> (InputColors, Vec<String>) {
        let mut resolved = InputColors::default();
        let mut warnings = Vec::new();
        for (key, value) in &self.colors {
            match (key.as_str(), parse_color(value)) {
                (SUGGESTION_KEY, Some(parsed)) => resolved.suggestion = parsed,
                (SUGGESTION_KEY, None) => warnings.push(format!(
                    "[theme.input] {key} = \"{value}\" is not a color; using the default"
                )),
                _ => warnings.push(format!("[theme.input] unknown key \"{key}\"; ignored")),
            }
        }
        (resolved, warnings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme(pairs: &[(&str, &str)]) -> InputTheme {
        InputTheme {
            colors: pairs
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
        }
    }

    #[test]
    fn missing_table_gives_the_default_cyan() {
        let (colors, warnings) = InputTheme::default().resolve();
        assert!(warnings.is_empty());
        assert_eq!(colors, InputColors::default());
        assert_eq!(colors.suggestion, Color::Cyan);
    }

    #[test]
    fn suggestion_key_overrides_the_color() {
        let (colors, warnings) = theme(&[("suggestion", "#112233")]).resolve();
        assert!(warnings.is_empty());
        assert_eq!(
            colors.suggestion,
            Color::Rgb {
                r: 0x11,
                g: 0x22,
                b: 0x33
            }
        );
    }

    #[test]
    fn bad_color_and_unknown_key_warn_and_keep_the_default() {
        let (colors, warnings) = theme(&[("suggestion", "nope"), ("hint", "red")]).resolve();
        assert_eq!(colors, InputColors::default());
        assert_eq!(
            warnings,
            [
                "[theme.input] unknown key \"hint\"; ignored",
                "[theme.input] suggestion = \"nope\" is not a color; using the default"
            ]
        );
    }
}
