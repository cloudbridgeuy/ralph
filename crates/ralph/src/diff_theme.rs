use crate::theme_slots::{resolve_slots, ResolveColors};
use ralph_server::config::theme::DiffTheme;
use termimad::crossterm::style::Color;

const SECTION: &str = "theme.diff";
const ADD_FG_DEFAULT: Color = Color::DarkGreen;
const DEL_FG_DEFAULT: Color = Color::DarkRed;
const HUNK_FG_DEFAULT: Color = Color::DarkCyan;
const ADD_BG_DEFAULT: Color = Color::Rgb {
    r: 45,
    g: 130,
    b: 45,
};
const DEL_BG_DEFAULT: Color = Color::Rgb {
    r: 140,
    g: 45,
    b: 45,
};
pub(crate) const DEFAULTS: [(&str, Color); 5] = [
    ("add_fg", ADD_FG_DEFAULT),
    ("del_fg", DEL_FG_DEFAULT),
    ("hunk_fg", HUNK_FG_DEFAULT),
    ("add_bg", ADD_BG_DEFAULT),
    ("del_bg", DEL_BG_DEFAULT),
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiffColors {
    pub add_fg: Color,
    pub del_fg: Color,
    pub hunk_fg: Color,
    pub add_bg: Color,
    pub del_bg: Color,
}

impl Default for DiffColors {
    fn default() -> Self {
        Self {
            add_fg: ADD_FG_DEFAULT,
            del_fg: DEL_FG_DEFAULT,
            hunk_fg: HUNK_FG_DEFAULT,
            add_bg: ADD_BG_DEFAULT,
            del_bg: DEL_BG_DEFAULT,
        }
    }
}

impl DiffColors {
    fn from_slots(slots: &[Color]) -> Option<Self> {
        match slots {
            [add_fg, del_fg, hunk_fg, add_bg, del_bg] => Some(Self {
                add_fg: *add_fg,
                del_fg: *del_fg,
                hunk_fg: *hunk_fg,
                add_bg: *add_bg,
                del_bg: *del_bg,
            }),
            _ => None,
        }
    }
}

#[cfg(test)]
pub(crate) fn test_colors() -> DiffColors {
    DiffColors {
        add_fg: Color::Blue,
        del_fg: Color::Magenta,
        hunk_fg: Color::Yellow,
        add_bg: Color::Rgb { r: 1, g: 2, b: 3 },
        del_bg: Color::Rgb { r: 4, g: 5, b: 6 },
    }
}

impl ResolveColors for DiffTheme {
    type Colors = DiffColors;

    fn resolve(&self) -> (DiffColors, Vec<String>) {
        let (slots, warnings) = resolve_slots(SECTION, &DEFAULTS, &self.colors);
        (DiffColors::from_slots(&slots).unwrap_or_default(), warnings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme(pairs: &[(&str, &str)]) -> DiffTheme {
        DiffTheme {
            colors: pairs
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
        }
    }

    #[test]
    fn add_bg_override_changes_the_resolved_color() {
        let (colors, warnings) = theme(&[("add_bg", "#000000")]).resolve();
        assert!(warnings.is_empty());
        assert_eq!(colors.add_bg, Color::Rgb { r: 0, g: 0, b: 0 });
        assert_eq!(colors.del_bg, DiffColors::default().del_bg);
    }

    #[test]
    fn bad_value_warns_with_the_section_name() {
        let (colors, warnings) = theme(&[("add_bg", "nope")]).resolve();
        assert_eq!(colors, DiffColors::default());
        assert_eq!(
            warnings,
            ["[theme.diff] add_bg = \"nope\" is not a color; using the default"]
        );
    }

    #[test]
    fn defaults_keep_todays_escape_bytes() {
        use crate::ansi::{bg, fg, CYAN, GREEN, GREEN_BG, RED, RED_BG};
        let colors = DiffColors::default();
        assert_eq!(fg(Some(colors.add_fg)), GREEN);
        assert_eq!(fg(Some(colors.del_fg)), RED);
        assert_eq!(fg(Some(colors.hunk_fg)), CYAN);
        assert_eq!(bg(Some(colors.add_bg)), GREEN_BG);
        assert_eq!(bg(Some(colors.del_bg)), RED_BG);
    }

    #[test]
    fn missing_table_gives_the_defaults_without_warnings() {
        let (colors, warnings) = DiffTheme::default().resolve();
        assert_eq!(colors, DiffColors::default());
        assert!(warnings.is_empty());
    }

    #[test]
    fn every_key_overrides_its_own_slot() {
        let (colors, warnings) = theme(&[
            ("add_fg", "#010101"),
            ("del_fg", "#020202"),
            ("hunk_fg", "#030303"),
            ("add_bg", "#040404"),
            ("del_bg", "#050505"),
        ])
        .resolve();
        let rgb = |n| Color::Rgb { r: n, g: n, b: n };
        assert!(warnings.is_empty());
        assert_eq!(
            colors,
            DiffColors {
                add_fg: rgb(1),
                del_fg: rgb(2),
                hunk_fg: rgb(3),
                add_bg: rgb(4),
                del_bg: rgb(5),
            }
        );
    }

    #[test]
    fn unknown_key_warns_and_is_ignored() {
        let (colors, warnings) = theme(&[("ctx_fg", "red")]).resolve();
        assert_eq!(colors, DiffColors::default());
        assert_eq!(warnings, ["[theme.diff] unknown key \"ctx_fg\"; ignored"]);
    }
}
