use crate::config::terminal_rgb;
use crate::markdown::{Palette, RolePalette, ThemeRole};
use crate::theme_slots::{resolve_slots, SlotTable};
use crate::{diff_theme, input_theme, status_theme, tool_theme, ui_theme};
use ralph_api::{tokens_css, ColorSpec, Rgb, Slot, TokensInput};
use ralph_server::config::{AppConfig, ThemeSection};
use std::collections::BTreeMap;
use termimad::crossterm::style::Color;

const ANSI_ENTRIES: usize = 16;
const FIRST_EXTENDED: u8 = 16;

pub(crate) fn color_spec(color: Color) -> Option<ColorSpec> {
    let named = |index: u8| Some(ColorSpec::Ansi(index));
    match color {
        Color::Rgb { r, g, b } => Some(ColorSpec::Rgb(Rgb { r, g, b })),
        Color::AnsiValue(n) if n < FIRST_EXTENDED => named(n),
        Color::AnsiValue(_) => {
            let [r, g, b, _] = terminal_rgb(color);
            Some(ColorSpec::Rgb(Rgb { r, g, b }))
        }
        Color::Reset => None,
        Color::Black => named(0),
        Color::DarkRed => named(1),
        Color::DarkGreen => named(2),
        Color::DarkYellow => named(3),
        Color::DarkBlue => named(4),
        Color::DarkMagenta => named(5),
        Color::DarkCyan => named(6),
        Color::Grey => named(7),
        Color::DarkGrey => named(8),
        Color::Red => named(9),
        Color::Green => named(10),
        Color::Yellow => named(11),
        Color::Blue => named(12),
        Color::Magenta => named(13),
        Color::Cyan => named(14),
        Color::White => named(15),
    }
}

fn palette_entries(palette: &Palette) -> [(&'static str, Option<Color>); 22] {
    [
        ("heading1", palette.headings[0]),
        ("heading2", palette.headings[1]),
        ("heading3", palette.headings[2]),
        ("bold", palette.bold),
        ("italic", palette.italic),
        ("inline-code", palette.inline_code),
        ("inline-code-bg", palette.inline_code_bg),
        ("bullet", palette.bullet),
        ("quote", palette.quote),
        ("table", palette.table),
        ("link", palette.link),
        ("url", palette.url),
        ("memory-ref", palette.memory_ref),
        ("border", palette.border),
        ("done", palette.done),
        ("blocked", palette.blocked),
        ("waiting", palette.waiting),
        ("pill-ok", palette.pill_ok),
        ("pill-bad", palette.pill_bad),
        ("pill-warn", palette.pill_warn),
        ("pill-info", palette.pill_info),
        ("pill-accent", palette.pill_accent),
    ]
}

fn palette_slots(palette: &Palette) -> Vec<Slot> {
    palette_entries(palette)
        .into_iter()
        .filter_map(|(name, color)| {
            Some(Slot {
                name: name.to_string(),
                color: color_spec(color?)?,
            })
        })
        .collect()
}

fn group_slots(
    group: &str,
    section: &str,
    defaults: &SlotTable,
    user: &BTreeMap<String, String>,
) -> Vec<Slot> {
    let (colors, _) = resolve_slots(section, defaults, user);
    defaults
        .iter()
        .zip(colors)
        .filter_map(|((key, _), color)| {
            Some(Slot {
                name: format!("{group}-{}", key.replace('_', "-")),
                color: color_spec(color)?,
            })
        })
        .collect()
}

pub(crate) fn theme_slots(theme: &ThemeSection) -> Vec<Slot> {
    let (palette, _) = theme.assistant.palette(ThemeRole::Assistant);
    let mut slots = palette_slots(&palette);
    slots.extend(group_slots(
        "status",
        "theme.status",
        &status_theme::DEFAULTS,
        &theme.status.colors,
    ));
    slots.extend(group_slots(
        "diff",
        "theme.diff",
        &diff_theme::DEFAULTS,
        &theme.diff.colors,
    ));
    slots.extend(group_slots(
        "input",
        "theme.input",
        &input_theme::DEFAULTS,
        &theme.input.colors,
    ));
    slots.extend(group_slots(
        "tools",
        "theme.tools",
        &tool_theme::DEFAULTS,
        &theme.tools.colors,
    ));
    slots.extend(group_slots(
        "ui",
        "theme.ui",
        &ui_theme::DEFAULTS,
        &theme.ui.colors,
    ));
    slots
}

pub(crate) fn ansi_table(entries: &[String]) -> Result<[Rgb; 16], String> {
    let parsed = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            Rgb::from_hex(entry).ok_or_else(|| format!("entry {index} \"{entry}\" is not #rrggbb"))
        })
        .collect::<Result<Vec<Rgb>, String>>()?;
    let count = parsed.len();
    <[Rgb; 16]>::try_from(parsed)
        .map_err(|_| format!("expected {ANSI_ENTRIES} entries, got {count}"))
}

pub(crate) fn render_tokens(config: &AppConfig) -> Result<String, String> {
    let ansi_dark =
        ansi_table(&config.web.ansi_dark).map_err(|e| format!("[web] ansi_dark: {e}"))?;
    let ansi_light =
        ansi_table(&config.web.ansi_light).map_err(|e| format!("[web] ansi_light: {e}"))?;
    let slots = theme_slots(&config.theme);
    Ok(tokens_css(&TokensInput {
        font: &config.web.font,
        ansi_dark: &ansi_dark,
        ansi_light: &ansi_light,
        slots: &slots,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_server::config::WebSection;
    use std::collections::BTreeMap;

    fn find<'a>(slots: &'a [Slot], name: &str) -> Option<&'a ColorSpec> {
        slots
            .iter()
            .find(|slot| slot.name == name)
            .map(|slot| &slot.color)
    }

    fn position(slots: &[Slot], name: &str) -> Option<usize> {
        slots.iter().position(|slot| slot.name == name)
    }

    fn hex(color: Color) -> Option<String> {
        match color_spec(color) {
            Some(ColorSpec::Rgb(rgb)) => Some(rgb.to_string()),
            _ => None,
        }
    }

    #[test]
    fn named_colors_map_to_their_ansi_index() {
        let table = [
            (Color::Black, 0),
            (Color::DarkRed, 1),
            (Color::DarkGreen, 2),
            (Color::DarkYellow, 3),
            (Color::DarkBlue, 4),
            (Color::DarkMagenta, 5),
            (Color::DarkCyan, 6),
            (Color::Grey, 7),
            (Color::DarkGrey, 8),
            (Color::Red, 9),
            (Color::Green, 10),
            (Color::Yellow, 11),
            (Color::Blue, 12),
            (Color::Magenta, 13),
            (Color::Cyan, 14),
            (Color::White, 15),
        ];
        for (color, index) in table {
            assert_eq!(color_spec(color), Some(ColorSpec::Ansi(index)), "{color:?}");
        }
    }

    #[test]
    fn reset_has_no_variable() {
        assert_eq!(color_spec(Color::Reset), None);
    }

    #[test]
    fn rgb_passes_through() {
        assert_eq!(
            color_spec(Color::Rgb { r: 1, g: 2, b: 3 }),
            Some(ColorSpec::Rgb(Rgb { r: 1, g: 2, b: 3 }))
        );
    }

    #[test]
    fn ansi_values_below_sixteen_stay_variables() {
        assert_eq!(color_spec(Color::AnsiValue(0)), Some(ColorSpec::Ansi(0)));
        assert_eq!(color_spec(Color::AnsiValue(15)), Some(ColorSpec::Ansi(15)));
    }

    #[test]
    fn cube_and_grey_ramps_resolve_to_rgb() {
        assert_eq!(hex(Color::AnsiValue(16)).as_deref(), Some("#000000"));
        assert_eq!(hex(Color::AnsiValue(196)).as_deref(), Some("#ff0000"));
        assert_eq!(hex(Color::AnsiValue(231)).as_deref(), Some("#ffffff"));
        assert_eq!(hex(Color::AnsiValue(232)).as_deref(), Some("#080808"));
        assert_eq!(hex(Color::AnsiValue(255)).as_deref(), Some("#eeeeee"));
        assert_eq!(hex(Color::AnsiValue(21)).as_deref(), Some("#0000ff"));
    }

    #[test]
    fn default_link_is_ansi_blue() {
        let slots = theme_slots(&ThemeSection::default());
        assert_eq!(find(&slots, "link"), Some(&ColorSpec::Ansi(12)));
    }

    #[test]
    fn default_diff_background_is_rgb() {
        let slots = theme_slots(&ThemeSection::default());
        assert!(matches!(
            find(&slots, "diff-add-bg"),
            Some(ColorSpec::Rgb(_))
        ));
    }

    #[test]
    fn user_palette_override_wins() {
        let mut theme = ThemeSection::default();
        theme
            .assistant
            .colors
            .insert("link".to_string(), "#112233".to_string());
        let slots = theme_slots(&theme);
        assert_eq!(
            find(&slots, "link"),
            Some(&ColorSpec::Rgb(Rgb {
                r: 0x11,
                g: 0x22,
                b: 0x33
            }))
        );
    }

    #[test]
    fn user_group_override_wins() {
        let mut theme = ThemeSection::default();
        theme.ui.colors = BTreeMap::from([("spinner".to_string(), "#010203".to_string())]);
        let slots = theme_slots(&theme);
        assert_eq!(
            find(&slots, "ui-spinner"),
            Some(&ColorSpec::Rgb(Rgb { r: 1, g: 2, b: 3 }))
        );
    }

    #[test]
    fn slots_are_ordered_palette_then_groups() {
        let slots = theme_slots(&ThemeSection::default());
        let order: Vec<usize> = [
            "heading1",
            "pill-accent",
            "status-mode-idle-fg",
            "diff-add-fg",
            "input-suggestion",
            "tools-name",
            "ui-spinner",
        ]
        .iter()
        .filter_map(|name| position(&slots, name))
        .collect();
        assert_eq!(order.len(), 7);
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn palette_slots_use_kebab_names_in_order() {
        let slots = theme_slots(&ThemeSection::default());
        let names: Vec<&str> = slots.iter().map(|slot| slot.name.as_str()).collect();
        let heading = names.iter().position(|name| *name == "heading1");
        assert_eq!(heading, Some(0));
        assert!(position(&slots, "inline-code-bg").is_some());
        assert!(position(&slots, "memory-ref").is_some());
        assert!(position(&slots, "pill-ok").is_some());
    }

    #[test]
    fn reset_slots_are_absent() {
        let slots = theme_slots(&ThemeSection::default());
        assert_eq!(find(&slots, "status-store-fg"), None);
        assert!(find(&slots, "status-mode-idle-fg").is_some());
    }

    #[test]
    fn campbell_default_parses() {
        let table = ansi_table(&WebSection::default().ansi_dark).unwrap();
        assert_eq!(table[0].to_string(), "#0c0c0c");
        assert_eq!(table[15].to_string(), "#f2f2f2");
    }

    #[test]
    fn wrong_length_is_an_error() {
        let entries = vec!["#000000".to_string(); 15];
        assert!(ansi_table(&entries).is_err());
    }

    #[test]
    fn bad_entry_names_its_index() {
        let mut entries = vec!["#000000".to_string(); 16];
        entries[3] = "red".to_string();
        let error = ansi_table(&entries).unwrap_err();
        assert!(error.contains('3'), "{error}");
        assert!(error.contains("red"), "{error}");
    }

    #[test]
    fn default_config_renders_tokens() {
        let css = render_tokens(&AppConfig::default()).unwrap();
        assert!(css.starts_with(
            ":root {\n  --r-font: ui-monospace, Menlo, monospace;\n  --r-ansi-0: #0c0c0c;"
        ));
        assert!(css.contains("@media (prefers-color-scheme: light)"));
        assert!(css.contains("--r-link: var(--r-ansi-12);"));
    }

    #[test]
    fn bad_web_table_is_an_error() {
        let mut config = AppConfig::default();
        config.web.ansi_dark.pop();
        let error = render_tokens(&config).unwrap_err();
        assert!(error.contains("ansi_dark"), "{error}");
    }
}
