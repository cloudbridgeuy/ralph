use crate::ansi::fg;
use crate::theme_slots::{resolve_slots, ResolveColors};
pub use ralph_server::config::theme::ToolTheme;
use termimad::crossterm::style::Color;

const SECTION: &str = "theme.tools";
const NAME_DEFAULT: Color = Color::DarkCyan;
const ARGS_DEFAULT: Color = Color::DarkGrey;
const LABEL_DEFAULT: Color = Color::DarkGrey;
const OK_DEFAULT: Color = Color::DarkGreen;
const ERROR_DEFAULT: Color = Color::DarkRed;
const ACCENT_DEFAULT: Color = Color::DarkYellow;
const MATCH_PATH_DEFAULT: Color = Color::DarkGrey;
const MATCH_TEXT_DEFAULT: Color = Color::Yellow;
pub(crate) const DEFAULTS: [(&str, Color); 8] = [
    ("name", NAME_DEFAULT),
    ("args", ARGS_DEFAULT),
    ("label", LABEL_DEFAULT),
    ("ok", OK_DEFAULT),
    ("error", ERROR_DEFAULT),
    ("accent", ACCENT_DEFAULT),
    ("match_path", MATCH_PATH_DEFAULT),
    ("match_text", MATCH_TEXT_DEFAULT),
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToolColors {
    pub name: Color,
    pub args: Color,
    pub label: Color,
    pub ok: Color,
    pub error: Color,
    pub accent: Color,
    pub match_path: Color,
    pub match_text: Color,
}

impl Default for ToolColors {
    fn default() -> Self {
        Self {
            name: NAME_DEFAULT,
            args: ARGS_DEFAULT,
            label: LABEL_DEFAULT,
            ok: OK_DEFAULT,
            error: ERROR_DEFAULT,
            accent: ACCENT_DEFAULT,
            match_path: MATCH_PATH_DEFAULT,
            match_text: MATCH_TEXT_DEFAULT,
        }
    }
}

impl ToolColors {
    fn from_slots(slots: &[Color]) -> Option<Self> {
        match slots {
            [name, args, label, ok, error, accent, match_path, match_text] => Some(Self {
                name: *name,
                args: *args,
                label: *label,
                ok: *ok,
                error: *error,
                accent: *accent,
                match_path: *match_path,
                match_text: *match_text,
            }),
            _ => None,
        }
    }

    pub fn name_fg(&self) -> String {
        fg(Some(self.name))
    }

    pub fn args_fg(&self) -> String {
        fg(Some(self.args))
    }

    pub fn label_fg(&self) -> String {
        fg(Some(self.label))
    }

    pub fn ok_fg(&self) -> String {
        fg(Some(self.ok))
    }

    pub fn error_fg(&self) -> String {
        fg(Some(self.error))
    }

    pub fn accent_fg(&self) -> String {
        fg(Some(self.accent))
    }

    pub fn match_path_fg(&self) -> String {
        fg(Some(self.match_path))
    }

    pub fn match_text_fg(&self) -> String {
        fg(Some(self.match_text))
    }
}

#[cfg(test)]
pub(crate) fn test_colors() -> ToolColors {
    ToolColors {
        name: Color::Blue,
        args: Color::Magenta,
        label: Color::Cyan,
        ok: Color::White,
        error: Color::Green,
        accent: Color::Red,
        match_path: Color::Black,
        match_text: Color::Grey,
    }
}

impl ResolveColors for ToolTheme {
    type Colors = ToolColors;

    fn resolve(&self) -> (ToolColors, Vec<String>) {
        let (slots, warnings) = resolve_slots(SECTION, &DEFAULTS, &self.colors);
        (ToolColors::from_slots(&slots).unwrap_or_default(), warnings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme(pairs: &[(&str, &str)]) -> ToolTheme {
        ToolTheme {
            colors: pairs
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
        }
    }

    #[test]
    fn override_changes_only_its_slot() {
        let (colors, warnings) = theme(&[("ok", "#000000")]).resolve();
        assert!(warnings.is_empty());
        assert_eq!(colors.ok, Color::Rgb { r: 0, g: 0, b: 0 });
        assert_eq!(colors.error, ToolColors::default().error);
    }

    #[test]
    fn bad_value_warns_with_the_section_name() {
        let (colors, warnings) = theme(&[("name", "nope")]).resolve();
        assert_eq!(colors, ToolColors::default());
        assert_eq!(
            warnings,
            ["[theme.tools] name = \"nope\" is not a color; using the default"]
        );
    }

    #[test]
    fn defaults_keep_todays_escape_bytes() {
        use crate::ansi::{BRIGHT_YELLOW, CYAN, GREEN, GREY, RED, YELLOW};
        let colors = ToolColors::default();
        assert_eq!(colors.name_fg(), CYAN);
        assert_eq!(colors.args_fg(), GREY);
        assert_eq!(colors.label_fg(), GREY);
        assert_eq!(colors.ok_fg(), GREEN);
        assert_eq!(colors.error_fg(), RED);
        assert_eq!(colors.accent_fg(), YELLOW);
        assert_eq!(colors.match_path_fg(), GREY);
        assert_eq!(colors.match_text_fg(), BRIGHT_YELLOW);
    }

    #[test]
    fn missing_table_gives_the_defaults_without_warnings() {
        let (colors, warnings) = ToolTheme::default().resolve();
        assert_eq!(colors, ToolColors::default());
        assert!(warnings.is_empty());
    }

    #[test]
    fn every_key_overrides_its_own_slot() {
        let (colors, warnings) = theme(&[
            ("name", "#010101"),
            ("args", "#020202"),
            ("label", "#030303"),
            ("ok", "#040404"),
            ("error", "#050505"),
            ("accent", "#060606"),
            ("match_path", "#070707"),
            ("match_text", "#080808"),
        ])
        .resolve();
        let rgb = |n| Color::Rgb { r: n, g: n, b: n };
        assert!(warnings.is_empty());
        assert_eq!(
            colors,
            ToolColors {
                name: rgb(1),
                args: rgb(2),
                label: rgb(3),
                ok: rgb(4),
                error: rgb(5),
                accent: rgb(6),
                match_path: rgb(7),
                match_text: rgb(8),
            }
        );
    }

    #[test]
    fn unknown_key_warns_and_is_ignored() {
        let (colors, warnings) = theme(&[("bold", "red")]).resolve();
        assert_eq!(colors, ToolColors::default());
        assert_eq!(warnings, ["[theme.tools] unknown key \"bold\"; ignored"]);
    }

    #[test]
    fn name_fg_matches_the_slot_color() {
        let colors = test_colors();
        assert_eq!(colors.name_fg(), "\x1b[94m");
        assert_eq!(colors.args_fg(), "\x1b[95m");
        assert_eq!(colors.label_fg(), "\x1b[96m");
        assert_eq!(colors.ok_fg(), "\x1b[97m");
        assert_eq!(colors.error_fg(), "\x1b[92m");
        assert_eq!(colors.accent_fg(), "\x1b[91m");
        assert_eq!(colors.match_path_fg(), "\x1b[30m");
        assert_eq!(colors.match_text_fg(), "\x1b[37m");
    }
}
