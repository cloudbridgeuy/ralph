use crate::markdown::parse_color;
use std::collections::BTreeMap;
use termimad::crossterm::style::Color;

pub type SlotTable = [(&'static str, Color)];

pub fn resolve_slots(
    section: &str,
    defaults: &SlotTable,
    user: &BTreeMap<String, String>,
) -> (Vec<Color>, Vec<String>) {
    let mut colors: Vec<Color> = defaults.iter().map(|(_, color)| *color).collect();
    let mut warnings = Vec::new();
    for (key, value) in user {
        let slot = defaults.iter().position(|(name, _)| name == key);
        match (slot, parse_color(value)) {
            (Some(at), Some(parsed)) => colors[at] = parsed,
            (Some(_), None) => warnings.push(format!(
                "[{section}] {key} = \"{value}\" is not a color; using the default"
            )),
            (None, _) => warnings.push(format!("[{section}] unknown key \"{key}\"; ignored")),
        }
    }
    (colors, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: [(&str, Color); 2] = [("fg", Color::Black), ("bg", Color::AnsiValue(236))];

    fn user(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn empty_user_table_gives_the_defaults() {
        let (colors, warnings) = resolve_slots("theme.x", &TABLE, &user(&[]));
        assert_eq!(colors, [Color::Black, Color::AnsiValue(236)]);
        assert!(warnings.is_empty());
    }

    #[test]
    fn override_replaces_only_the_given_slot() {
        let (colors, warnings) = resolve_slots("theme.x", &TABLE, &user(&[("fg", "#010203")]));
        assert_eq!(
            colors,
            [Color::Rgb { r: 1, g: 2, b: 3 }, Color::AnsiValue(236)]
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn bad_value_warns_and_keeps_the_default() {
        let (colors, warnings) = resolve_slots("theme.x", &TABLE, &user(&[("bg", "nope")]));
        assert_eq!(colors, [Color::Black, Color::AnsiValue(236)]);
        assert_eq!(
            warnings,
            ["[theme.x] bg = \"nope\" is not a color; using the default"]
        );
    }

    #[test]
    fn unknown_key_warns_and_is_ignored() {
        let (colors, warnings) = resolve_slots("theme.x", &TABLE, &user(&[("zz", "red")]));
        assert_eq!(colors, [Color::Black, Color::AnsiValue(236)]);
        assert_eq!(warnings, ["[theme.x] unknown key \"zz\"; ignored"]);
    }

    #[test]
    fn ansi_number_is_accepted() {
        let (colors, warnings) = resolve_slots("theme.x", &TABLE, &user(&[("fg", "17")]));
        assert_eq!(colors, [Color::AnsiValue(17), Color::AnsiValue(236)]);
        assert!(warnings.is_empty());
    }

    #[test]
    fn out_of_range_number_warns() {
        let (colors, warnings) = resolve_slots("theme.x", &TABLE, &user(&[("fg", "256")]));
        assert_eq!(colors[0], Color::Black);
        assert_eq!(warnings.len(), 1);
    }
}
