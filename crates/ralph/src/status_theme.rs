use crate::theme_slots::resolve_slots;
use ralph_core::status_segments::Shade;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use termimad::crossterm::style::Color;

const SECTION: &str = "theme.status";

const SHADES: [Shade; 14] = [
    Shade::ModeIdleFg,
    Shade::ModeIdleBg,
    Shade::ModeHoldFg,
    Shade::ModeHoldBg,
    Shade::ModeNapFg,
    Shade::ModeNapBg,
    Shade::ModeInboxFg,
    Shade::ModeInboxBg,
    Shade::ModeWorkFg,
    Shade::ModeWorkBg,
    Shade::StoreFg,
    Shade::WorkersFg,
    Shade::WatchesFg,
    Shade::BaseBg,
];

const DEFAULTS: [(&str, Color); 14] = [
    ("mode_idle_fg", Color::Black),
    ("mode_idle_bg", Color::Green),
    ("mode_hold_fg", Color::Black),
    ("mode_hold_bg", Color::Cyan),
    ("mode_nap_fg", Color::Black),
    ("mode_nap_bg", Color::Magenta),
    ("mode_inbox_fg", Color::Black),
    ("mode_inbox_bg", Color::Yellow),
    ("mode_work_fg", Color::White),
    ("mode_work_bg", Color::Blue),
    ("store_fg", Color::Reset),
    ("workers_fg", Color::Reset),
    ("watches_fg", Color::Reset),
    ("base_bg", Color::AnsiValue(236)),
];

#[derive(Debug, Clone, PartialEq)]
pub struct StatusColors {
    colors: Vec<Color>,
}

impl Default for StatusColors {
    fn default() -> Self {
        Self {
            colors: DEFAULTS.iter().map(|(_, color)| *color).collect(),
        }
    }
}

impl StatusColors {
    pub fn color(&self, shade: Shade) -> Color {
        SHADES
            .iter()
            .position(|slot| *slot == shade)
            .and_then(|at| self.colors.get(at))
            .copied()
            .unwrap_or(Color::Reset)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StatusTheme {
    #[serde(flatten)]
    pub colors: BTreeMap<String, String>,
}

impl StatusTheme {
    pub fn resolve(&self) -> (StatusColors, Vec<String>) {
        let (colors, warnings) = resolve_slots(SECTION, &DEFAULTS, &self.colors);
        (StatusColors { colors }, warnings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme(pairs: &[(&str, &str)]) -> StatusTheme {
        StatusTheme {
            colors: pairs
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
        }
    }

    #[test]
    fn missing_table_gives_the_defaults() {
        let (colors, warnings) = StatusTheme::default().resolve();
        assert!(warnings.is_empty());
        assert_eq!(colors, StatusColors::default());
        assert_eq!(colors.color(Shade::ModeIdleBg), Color::Green);
        assert_eq!(colors.color(Shade::ModeNapBg), Color::Magenta);
        assert_eq!(colors.color(Shade::ModeInboxBg), Color::Yellow);
        assert_eq!(colors.color(Shade::ModeWorkBg), Color::Blue);
        assert_eq!(colors.color(Shade::BaseBg), Color::AnsiValue(236));
        assert_eq!(colors.color(Shade::ModeIdleFg), Color::Black);
        assert_eq!(colors.color(Shade::StoreFg), Color::Reset);
    }

    #[test]
    fn every_slot_has_a_distinct_key_and_shade() {
        for at in 0..DEFAULTS.len() {
            assert!(DEFAULTS[..at].iter().all(|(k, _)| *k != DEFAULTS[at].0));
            assert!(SHADES[..at].iter().all(|s| *s != SHADES[at]));
        }
    }

    #[test]
    fn mode_backgrounds_are_distinct_from_each_other() {
        let (colors, _) = StatusTheme::default().resolve();
        let shades = [
            Shade::ModeIdleBg,
            Shade::ModeHoldBg,
            Shade::ModeNapBg,
            Shade::ModeInboxBg,
            Shade::ModeWorkBg,
        ];
        for (at, shade) in shades.iter().enumerate() {
            assert!(shades[..at]
                .iter()
                .all(|other| colors.color(*other) != colors.color(*shade)));
        }
    }

    #[test]
    fn partial_table_overrides_only_the_given_keys() {
        let (colors, warnings) =
            theme(&[("mode_nap_bg", "#112233"), ("mode_work_fg", "black")]).resolve();
        assert!(warnings.is_empty());
        assert_eq!(
            colors.color(Shade::ModeNapBg),
            Color::Rgb {
                r: 0x11,
                g: 0x22,
                b: 0x33
            }
        );
        assert_eq!(colors.color(Shade::ModeWorkFg), Color::Black);
        assert_eq!(colors.color(Shade::ModeIdleBg), Color::Green);
    }

    #[test]
    fn mode_override_recolours_the_mail_block() {
        use ralph_core::chat_status::Status;
        use ralph_core::status_segments::{bar, Context, Separator};
        let (colors, _) = theme(&[("mode_inbox_bg", "#112233")]).resolve();
        let status = Status {
            unread: 1,
            ..Status::default()
        };
        let spans = bar(&status, &Context::default(), 80, Separator::Block);
        let mail = spans.iter().find(|s| s.text.contains("unread")).unwrap();
        assert_eq!(
            colors.color(mail.bg),
            Color::Rgb {
                r: 0x11,
                g: 0x22,
                b: 0x33
            }
        );
    }

    #[test]
    fn the_removed_memory_fg_key_is_ignored_with_a_warning() {
        let (colors, warnings) = theme(&[("memory_fg", "red")]).resolve();
        assert_eq!(colors, StatusColors::default());
        assert_eq!(
            warnings,
            ["[theme.status] unknown key \"memory_fg\"; ignored"]
        );
    }

    #[test]
    fn bad_color_and_unknown_key_warn_and_keep_defaults() {
        let (colors, warnings) = theme(&[("mode_work_bg", "nope"), ("branch_fg", "red")]).resolve();
        assert_eq!(colors, StatusColors::default());
        assert_eq!(
            warnings,
            [
                "[theme.status] unknown key \"branch_fg\"; ignored",
                "[theme.status] mode_work_bg = \"nope\" is not a color; using the default"
            ]
        );
    }
}
