use crate::markdown::parse_color;
use ralph_core::status_segments::Shade;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use termimad::crossterm::style::Color;

const SLOTS: [(&str, Shade, Color); 13] = [
    ("mode_idle_fg", Shade::ModeIdleFg, Color::Black),
    ("mode_idle_bg", Shade::ModeIdleBg, Color::Green),
    ("mode_nap_fg", Shade::ModeNapFg, Color::Black),
    ("mode_nap_bg", Shade::ModeNapBg, Color::Magenta),
    ("mode_inbox_fg", Shade::ModeInboxFg, Color::Black),
    ("mode_inbox_bg", Shade::ModeInboxBg, Color::Yellow),
    ("mode_work_fg", Shade::ModeWorkFg, Color::White),
    ("mode_work_bg", Shade::ModeWorkBg, Color::Blue),
    ("store_fg", Shade::StoreFg, Color::Reset),
    ("workers_fg", Shade::WorkersFg, Color::Reset),
    ("watches_fg", Shade::WatchesFg, Color::Reset),
    ("memory_fg", Shade::MemoryFg, Color::Reset),
    ("base_bg", Shade::BaseBg, Color::AnsiValue(236)),
];

#[derive(Debug, Clone, PartialEq)]
pub struct StatusColors {
    colors: [(&'static str, Shade, Color); 13],
}

impl Default for StatusColors {
    fn default() -> Self {
        Self { colors: SLOTS }
    }
}

impl StatusColors {
    pub fn color(&self, shade: Shade) -> Color {
        self.colors
            .iter()
            .find(|(_, slot, _)| *slot == shade)
            .map_or(Color::Reset, |(_, _, color)| *color)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StatusTheme {
    #[serde(flatten)]
    pub colors: BTreeMap<String, String>,
}

impl StatusTheme {
    pub fn resolve(&self) -> (StatusColors, Vec<String>) {
        let mut resolved = StatusColors::default();
        let mut warnings = Vec::new();
        for (key, value) in &self.colors {
            let slot = resolved.colors.iter_mut().find(|(name, _, _)| name == key);
            match (slot, parse_color(value)) {
                (Some((_, _, color)), Some(parsed)) => *color = parsed,
                (Some(_), None) => warnings.push(format!(
                    "[theme.status] {key} = \"{value}\" is not a color; using the default"
                )),
                (None, _) => {
                    warnings.push(format!("[theme.status] unknown key \"{key}\"; ignored"))
                }
            }
        }
        (resolved, warnings)
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
        for (at, (key, shade, _)) in SLOTS.iter().enumerate() {
            assert!(SLOTS[..at].iter().all(|(k, s, _)| k != key && s != shade));
        }
    }

    #[test]
    fn mode_backgrounds_are_distinct_from_each_other() {
        let (colors, _) = StatusTheme::default().resolve();
        let shades = [
            Shade::ModeIdleBg,
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
