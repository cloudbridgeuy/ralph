use crate::theme_slots::{resolve_slots, ResolveColors};
use ralph_server::config::theme::UiTheme;
use termimad::crossterm::style::Color;

const SECTION: &str = "theme.ui";
const SPINNER_DEFAULT: Color = Color::DarkCyan;
const ASK_HEADER_DEFAULT: Color = Color::DarkMagenta;
const ASK_PATH_DEFAULT: Color = Color::DarkCyan;
const ASK_OK_DEFAULT: Color = Color::DarkGreen;
const ASK_ERROR_DEFAULT: Color = Color::DarkRed;
const WARNING_DEFAULT: Color = Color::DarkYellow;
const RULE_DEFAULT: Color = Color::AnsiValue(238);
pub(crate) const DEFAULTS: [(&str, Color); 7] = [
    ("spinner", SPINNER_DEFAULT),
    ("ask_header", ASK_HEADER_DEFAULT),
    ("ask_path", ASK_PATH_DEFAULT),
    ("ask_ok", ASK_OK_DEFAULT),
    ("ask_error", ASK_ERROR_DEFAULT),
    ("warning", WARNING_DEFAULT),
    ("rule", RULE_DEFAULT),
];

pub const DEFAULT_WARNING_COLOR: Color = WARNING_DEFAULT;
pub const DEFAULT_NOTICE_COLOR: Color = ASK_OK_DEFAULT;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiColors {
    pub spinner: Color,
    pub ask_header: Color,
    pub ask_path: Color,
    pub ask_ok: Color,
    pub ask_error: Color,
    pub warning: Color,
    pub rule: Color,
}

impl Default for UiColors {
    fn default() -> Self {
        Self {
            spinner: SPINNER_DEFAULT,
            ask_header: ASK_HEADER_DEFAULT,
            ask_path: ASK_PATH_DEFAULT,
            ask_ok: ASK_OK_DEFAULT,
            ask_error: ASK_ERROR_DEFAULT,
            warning: WARNING_DEFAULT,
            rule: RULE_DEFAULT,
        }
    }
}

impl UiColors {
    fn from_slots(slots: &[Color]) -> Option<Self> {
        match slots {
            [spinner, ask_header, ask_path, ask_ok, ask_error, warning, rule] => Some(Self {
                spinner: *spinner,
                ask_header: *ask_header,
                ask_path: *ask_path,
                ask_ok: *ask_ok,
                ask_error: *ask_error,
                warning: *warning,
                rule: *rule,
            }),
            _ => None,
        }
    }
}

impl ResolveColors for UiTheme {
    type Colors = UiColors;

    fn resolve(&self) -> (UiColors, Vec<String>) {
        let (slots, warnings) = resolve_slots(SECTION, &DEFAULTS, &self.colors);
        (UiColors::from_slots(&slots).unwrap_or_default(), warnings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ansi::{fg, CYAN, GREEN, MAGENTA, RED, YELLOW};

    fn theme(pairs: &[(&str, &str)]) -> UiTheme {
        UiTheme {
            colors: pairs
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
        }
    }

    #[test]
    fn override_changes_only_its_slot() {
        let (colors, warnings) = theme(&[("warning", "#000000")]).resolve();
        assert!(warnings.is_empty());
        assert_eq!(colors.warning, Color::Rgb { r: 0, g: 0, b: 0 });
        assert_eq!(colors.spinner, UiColors::default().spinner);
    }

    #[test]
    fn bad_value_warns_with_the_section_name() {
        let (colors, warnings) = theme(&[("spinner", "nope")]).resolve();
        assert_eq!(colors, UiColors::default());
        assert_eq!(
            warnings,
            ["[theme.ui] spinner = \"nope\" is not a color; using the default"]
        );
    }

    #[test]
    fn unknown_key_warns_and_is_ignored() {
        let (colors, warnings) = theme(&[("bold", "red")]).resolve();
        assert_eq!(colors, UiColors::default());
        assert_eq!(warnings, ["[theme.ui] unknown key \"bold\"; ignored"]);
    }

    #[test]
    fn missing_table_gives_the_defaults_without_warnings() {
        let (colors, warnings) = UiTheme::default().resolve();
        assert_eq!(colors, UiColors::default());
        assert!(warnings.is_empty());
    }

    #[test]
    fn defaults_keep_todays_escape_bytes() {
        let colors = UiColors::default();
        assert_eq!(fg(Some(colors.spinner)), CYAN);
        assert_eq!(fg(Some(colors.ask_header)), MAGENTA);
        assert_eq!(fg(Some(colors.ask_path)), CYAN);
        assert_eq!(fg(Some(colors.ask_ok)), GREEN);
        assert_eq!(fg(Some(colors.ask_error)), RED);
        assert_eq!(fg(Some(colors.warning)), YELLOW);
        assert_eq!(fg(Some(colors.rule)), "\x1b[38;5;238m");
    }

    #[test]
    fn every_key_overrides_its_own_slot() {
        let (colors, warnings) = theme(&[
            ("spinner", "#010101"),
            ("ask_header", "#020202"),
            ("ask_path", "#030303"),
            ("ask_ok", "#040404"),
            ("ask_error", "#050505"),
            ("warning", "#060606"),
            ("rule", "#070707"),
        ])
        .resolve();
        let rgb = |n| Color::Rgb { r: n, g: n, b: n };
        assert!(warnings.is_empty());
        assert_eq!(
            colors,
            UiColors {
                spinner: rgb(1),
                ask_header: rgb(2),
                ask_path: rgb(3),
                ask_ok: rgb(4),
                ask_error: rgb(5),
                warning: rgb(6),
                rule: rgb(7),
            }
        );
    }
}
