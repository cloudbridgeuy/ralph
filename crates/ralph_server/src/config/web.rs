use serde::{Deserialize, Serialize};

const ANSI_SLOTS: usize = 16;

const CAMPBELL: [&str; ANSI_SLOTS] = [
    "#0c0c0c", "#c50f1f", "#13a10e", "#c19c00", "#0037da", "#881798", "#3a96dd", "#cccccc",
    "#767676", "#e74856", "#16c60c", "#f9f1a5", "#3b78ff", "#b4009e", "#61d6d6", "#f2f2f2",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebSection {
    #[serde(default = "default_font")]
    pub font: String,
    #[serde(default = "default_ansi_dark")]
    pub ansi_dark: Vec<String>,
    #[serde(default = "default_ansi_light")]
    pub ansi_light: Vec<String>,
}

fn default_font() -> String {
    "ui-monospace, Menlo, monospace".to_string()
}

fn default_ansi_dark() -> Vec<String> {
    CAMPBELL.iter().map(|hex| (*hex).to_string()).collect()
}

fn default_ansi_light() -> Vec<String> {
    let mut table = default_ansi_dark();
    table.swap(0, 7);
    table.swap(8, 15);
    table
}

impl Default for WebSection {
    fn default() -> Self {
        Self {
            font: default_font(),
            ansi_dark: default_ansi_dark(),
            ansi_light: default_ansi_light(),
        }
    }
}

fn is_hex_color(entry: &str) -> bool {
    entry.len() == 7 && entry.starts_with('#') && entry[1..].chars().all(|c| c.is_ascii_hexdigit())
}

fn validate_table(name: &str, table: &[String]) -> Result<(), String> {
    if table.len() != ANSI_SLOTS {
        return Err(format!(
            "[web] {name} must have {ANSI_SLOTS} entries, got {}",
            table.len()
        ));
    }
    match table.iter().enumerate().find(|(_, e)| !is_hex_color(e)) {
        Some((index, entry)) => Err(format!(
            "[web] {name}[{index}] = \"{entry}\" is not #rrggbb"
        )),
        None => Ok(()),
    }
}

fn font_is_plain(font: &str) -> bool {
    let mut open: Option<char> = None;
    for c in font.chars() {
        match (c, open) {
            ('"' | '\'', None) => open = Some(c),
            ('"' | '\'', Some(q)) if q == c => open = None,
            ('"' | '\'', Some(_)) => {}
            (c, _) if c.is_alphanumeric() || matches!(c, ' ' | ',' | '-' | '_' | '.') => {}
            _ => return false,
        }
    }
    open.is_none()
}

impl WebSection {
    pub fn validate(&self) -> Result<(), String> {
        if self.font.trim().is_empty() {
            return Err("[web] font must not be empty".to_string());
        }
        if !font_is_plain(&self.font) {
            return Err(format!(
                "[web] font = \"{}\" may only use letters, digits, space, ',', '-', '_', '.' and balanced quotes",
                self.font
            ));
        }
        validate_table("ansi_dark", &self.ansi_dark)?;
        validate_table("ansi_light", &self.ansi_light)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{leaf_sources, AppConfig, ConfigError, Source};
    use toml::Table;

    fn with_dark(table: Vec<String>) -> WebSection {
        WebSection {
            ansi_dark: table,
            ..WebSection::default()
        }
    }

    fn dark_with(index: usize, value: &str) -> WebSection {
        let mut table = default_ansi_dark();
        table[index] = value.to_string();
        with_dark(table)
    }

    fn dark_row_source(effective: &str, global: &str) -> (usize, Source) {
        let effective: Table = effective.parse().unwrap();
        let global: Table = global.parse().unwrap();
        let rows = leaf_sources(&effective, &global, &Table::new());
        let dark: Vec<_> = rows.iter().filter(|r| r.key == "web.ansi_dark").collect();
        (dark.len(), dark[0].source)
    }

    fn default_effective() -> String {
        toml::to_string(&AppConfig::default()).unwrap()
    }

    #[test]
    fn defaults_are_valid_with_sixteen_entries() {
        let web = WebSection::default();
        assert_eq!(web.validate(), Ok(()));
        assert_eq!(web.ansi_dark.len(), 16);
        assert_eq!(web.ansi_light.len(), 16);
        assert_eq!(web.font, "ui-monospace, Menlo, monospace");
    }

    #[test]
    fn dark_table_is_campbell() {
        let dark = WebSection::default().ansi_dark;
        assert_eq!(dark, CAMPBELL);
        assert_eq!(dark[0], "#0c0c0c");
        assert_eq!(dark[4], "#0037da");
        assert_eq!(dark[15], "#f2f2f2");
    }

    #[test]
    fn light_table_swaps_black_white_slots() {
        let web = WebSection::default();
        assert_eq!(web.ansi_light[0], "#cccccc");
        assert_eq!(web.ansi_light[7], "#0c0c0c");
        assert_eq!(web.ansi_light[8], "#f2f2f2");
        assert_eq!(web.ansi_light[15], "#767676");
        for slot in [1, 2, 3, 4, 5, 6, 9, 10, 11, 12, 13, 14] {
            assert_eq!(web.ansi_light[slot], web.ansi_dark[slot]);
        }
    }

    #[test]
    fn wrong_entry_counts_are_errors() {
        let mut short = default_ansi_dark();
        short.pop();
        assert_eq!(
            with_dark(short).validate(),
            Err("[web] ansi_dark must have 16 entries, got 15".to_string())
        );
        let mut long = default_ansi_dark();
        long.push("#000000".to_string());
        assert_eq!(
            with_dark(long).validate(),
            Err("[web] ansi_dark must have 16 entries, got 17".to_string())
        );
        let light = WebSection {
            ansi_light: Vec::new(),
            ..WebSection::default()
        };
        assert_eq!(
            light.validate(),
            Err("[web] ansi_light must have 16 entries, got 0".to_string())
        );
    }

    #[test]
    fn malformed_entries_are_errors() {
        for bad in ["red", "#12345", "#gggggg", "123456", "#1234567"] {
            assert_eq!(
                dark_with(3, bad).validate(),
                Err(format!("[web] ansi_dark[3] = \"{bad}\" is not #rrggbb"))
            );
        }
    }

    #[test]
    fn uppercase_hex_is_accepted() {
        assert_eq!(dark_with(1, "#ABCDEF").validate(), Ok(()));
    }

    #[test]
    fn empty_font_is_an_error() {
        let web = WebSection {
            font: "  ".to_string(),
            ..WebSection::default()
        };
        assert_eq!(
            web.validate(),
            Err("[web] font must not be empty".to_string())
        );
    }

    fn font(value: &str) -> WebSection {
        WebSection {
            font: value.to_string(),
            ..WebSection::default()
        }
    }

    #[test]
    fn fonts_that_break_the_stylesheet_are_errors() {
        for bad in [
            "Fira /* x",
            "foo(",
            "foo)",
            "a[0]",
            "\"Fira Code",
            "'Fira Code",
            "a; b",
            "a } b",
            "a: b",
            "a\\b",
        ] {
            let result = font(bad).validate();
            assert!(
                matches!(&result, Err(message) if message.starts_with("[web] font")),
                "{bad:?} gave {result:?}"
            );
        }
    }

    #[test]
    fn plain_and_quoted_font_lists_are_accepted() {
        for good in [
            "ui-monospace, Menlo, monospace",
            "Geist Mono, monospace",
            "\"Fira Code\", 'Menlo', monospace",
            "a_b.c-d",
        ] {
            assert_eq!(font(good).validate(), Ok(()), "{good:?}");
        }
    }

    #[test]
    fn bad_font_through_parse_is_invalid_web() {
        match AppConfig::parse_reporting_unknown("[web]\nfont = \"foo(\"\n", "c") {
            Err(ConfigError::InvalidWeb { message, .. }) => {
                assert!(message.starts_with("[web] font"), "{message}");
            }
            other => panic!("expected InvalidWeb, got {other:?}"),
        }
    }

    #[test]
    fn partial_section_keeps_default_tables() {
        let (config, unknown) =
            AppConfig::parse_reporting_unknown("[web]\nfont = \"X\"\n", "c").unwrap();
        assert!(unknown.is_empty());
        assert_eq!(config.web.font, "X");
        assert_eq!(config.web.ansi_dark, default_ansi_dark());
        assert_eq!(config.web.ansi_light, default_ansi_light());
    }

    #[test]
    fn bad_entry_through_parse_is_invalid_web() {
        let mut dark = default_ansi_dark();
        dark[3] = "x".to_string();
        let text = format!(
            "[web]\nansi_dark = [{}]\n",
            dark.iter()
                .map(|e| format!("\"{e}\""))
                .collect::<Vec<_>>()
                .join(", ")
        );
        match AppConfig::parse_reporting_unknown(&text, "c") {
            Err(ConfigError::InvalidWeb { message, .. }) => {
                assert_eq!(message, "[web] ansi_dark[3] = \"x\" is not #rrggbb");
            }
            other => panic!("expected InvalidWeb, got {other:?}"),
        }
    }

    #[test]
    fn unknown_web_key_is_reported() {
        let (_, unknown) = AppConfig::parse_reporting_unknown("[web]\nbogus = 1\n", "c").unwrap();
        assert_eq!(unknown, ["web.bogus"]);
    }

    #[test]
    fn leaf_sources_shows_one_default_row_when_unset() {
        let (count, source) = dark_row_source(&default_effective(), "");
        assert_eq!((count, source), (1, Source::Default));
    }

    #[test]
    fn leaf_sources_shows_global_when_layer_sets_table() {
        let global = "[web]\nansi_dark = [\"#000000\"]\n";
        let (count, source) = dark_row_source(global, global);
        assert_eq!((count, source), (1, Source::Global));
    }
}
