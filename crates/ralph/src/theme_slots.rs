use crate::markdown::parse_color;
use std::collections::BTreeMap;
use termimad::crossterm::style::Color;

pub type SlotTable = [(&'static str, Color)];

pub trait ResolveColors {
    type Colors;

    fn resolve(&self) -> (Self::Colors, Vec<String>);
}

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

#[cfg(test)]
mod docs_tests {
    use super::SlotTable;

    const DOCS: &str = include_str!("../../../docs/configuration.md");
    const MARKDOWN_SOURCE: &str = include_str!("markdown.rs");

    fn section<'a>(docs: &'a str, heading: &str) -> &'a str {
        let Some(start) = docs.find(heading) else {
            return "";
        };
        let body = &docs[start + heading.len()..];
        let end = body.find("\n#").unwrap_or(body.len());
        &body[..end]
    }

    fn is_documented(section: &str, key: &str) -> bool {
        let line_start = format!("\n{key} =");
        section.contains(&format!("`{key}`")) || section.contains(&line_start)
    }

    fn missing_keys<'a>(section: &str, keys: &[&'a str]) -> Vec<&'a str> {
        keys.iter()
            .copied()
            .filter(|key| !is_documented(section, key))
            .collect()
    }

    fn table_keys(table: &SlotTable) -> Vec<&'static str> {
        table.iter().map(|(key, _)| *key).collect()
    }

    fn palette_keys(source: &str) -> Vec<&str> {
        let Some(start) = source.find("fn slot(") else {
            return Vec::new();
        };
        let body = &source[start..];
        let body = &body[..body.find("_ => None").unwrap_or(body.len())];
        body.lines()
            .filter_map(|line| line.trim().strip_prefix('"'))
            .filter_map(|rest| rest.split_once('"').map(|(key, _)| key))
            .collect()
    }

    fn assert_documented(heading: &str, keys: &[&str]) {
        let missing = missing_keys(section(DOCS, heading), keys);
        assert!(
            missing.is_empty(),
            "docs/configuration.md under {heading} lacks keys: {missing:?}"
        );
    }

    #[test]
    fn section_stops_at_the_next_heading() {
        let docs = "#### A\n`x`\n#### B\n`y`\n";
        assert_eq!(section(docs, "#### A"), "\n`x`");
        assert_eq!(section(docs, "#### Z"), "");
    }

    #[test]
    fn documented_key_is_a_backticked_cell_or_a_toml_line() {
        assert!(is_documented("| `add_fg` | x |", "add_fg"));
        assert!(is_documented("\nadd_fg = \"red\"", "add_fg"));
        assert!(!is_documented("add_fg in prose", "add_fg"));
    }

    #[test]
    fn missing_keys_lists_only_the_absent_ones() {
        assert_eq!(missing_keys("`a` `b`", &["a", "c", "d"]), ["c", "d"]);
    }

    #[test]
    fn palette_keys_are_read_from_the_slot_match() {
        let source = "fn slot(&mut self) {\n match key {\n \"bold\" => x,\n \"link\" => y,\n _ => None,\n }\n}";
        assert_eq!(palette_keys(source), ["bold", "link"]);
        assert!(palette_keys(MARKDOWN_SOURCE).len() >= 17);
    }

    #[test]
    fn docs_list_every_status_key() {
        assert_documented(
            "#### Status line colors: `[theme.status]`",
            &table_keys(&crate::status_theme::DEFAULTS),
        );
    }

    #[test]
    fn docs_list_every_input_key() {
        assert_documented(
            "#### Input colors: `[theme.input]`",
            &table_keys(&crate::input_theme::DEFAULTS),
        );
    }

    #[test]
    fn docs_list_every_diff_key() {
        assert_documented(
            "#### Diff colors: `[theme.diff]`",
            &table_keys(&crate::diff_theme::DEFAULTS),
        );
    }

    #[test]
    fn docs_list_every_tools_key() {
        assert_documented(
            "#### Tool colors: `[theme.tools]`",
            &table_keys(&crate::tool_theme::DEFAULTS),
        );
    }

    #[test]
    fn docs_list_every_ui_key() {
        assert_documented(
            "#### UI colors: `[theme.ui]`",
            &table_keys(&crate::ui_theme::DEFAULTS),
        );
    }

    #[test]
    fn docs_list_every_palette_key() {
        assert_documented(
            "#### Per-role themes: `[theme.assistant]` and `[theme.agent]`",
            &palette_keys(MARKDOWN_SOURCE),
        );
    }
}
