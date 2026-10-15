use crate::markdown::parse_color;
use ralph_core::status_segments::Shade;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use termimad::crossterm::style::Color;

const SLOTS: [(&str, Shade, Color); 13] = [
    ("mode_fg", Shade::ModeFg, Color::Black),
    ("mode_bg", Shade::ModeBg, Color::Green),
    ("mail_fg", Shade::MailFg, Color::Black),
    ("mail_bg", Shade::MailBg, Color::DarkGreen),
    ("store_fg", Shade::StoreFg, Color::Reset),
    ("workers_fg", Shade::WorkersFg, Color::Reset),
    ("watches_fg", Shade::WatchesFg, Color::Reset),
    ("memory_fg", Shade::MemoryFg, Color::Reset),
    ("cache_fg", Shade::CacheFg, Color::Black),
    ("cache_bg", Shade::CacheBg, Color::DarkGreen),
    ("cost_fg", Shade::CostFg, Color::Black),
    ("cost_bg", Shade::CostBg, Color::Green),
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
        assert_eq!(colors.color(Shade::ModeBg), Color::Green);
        assert_eq!(colors.color(Shade::MailBg), Color::DarkGreen);
        assert_eq!(colors.color(Shade::BaseBg), Color::AnsiValue(236));
        assert_eq!(colors.color(Shade::ModeFg), Color::Black);
        assert_eq!(colors.color(Shade::StoreFg), Color::Reset);
    }

    #[test]
    fn every_slot_has_a_distinct_key_and_shade() {
        for (at, (key, shade, _)) in SLOTS.iter().enumerate() {
            assert!(SLOTS[..at].iter().all(|(k, s, _)| k != key && s != shade));
        }
    }

    #[test]
    fn partial_table_overrides_only_the_given_keys() {
        let (colors, warnings) = theme(&[("mode_bg", "#112233"), ("cost_fg", "white")]).resolve();
        assert!(warnings.is_empty());
        assert_eq!(
            colors.color(Shade::ModeBg),
            Color::Rgb {
                r: 0x11,
                g: 0x22,
                b: 0x33
            }
        );
        assert_eq!(colors.color(Shade::CostFg), Color::White);
        assert_eq!(colors.color(Shade::MailBg), Color::DarkGreen);
    }

    #[test]
    fn bad_color_and_unknown_key_warn_and_keep_defaults() {
        let (colors, warnings) = theme(&[("mode_bg", "nope"), ("branch_fg", "red")]).resolve();
        assert_eq!(colors, StatusColors::default());
        assert_eq!(
            warnings,
            [
                "[theme.status] unknown key \"branch_fg\"; ignored",
                "[theme.status] mode_bg = \"nope\" is not a color; using the default"
            ]
        );
    }
}
