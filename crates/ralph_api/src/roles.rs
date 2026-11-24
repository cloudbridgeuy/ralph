#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Theme {
    Dark,
    Light,
}

enum Pct {
    Lit(u8),
    Var(&'static str),
}

enum Value {
    Var(&'static str),
    Heading(&'static str),
    Mix {
        a: &'static str,
        pct: Pct,
        b: &'static str,
    },
}

struct Picks {
    bg: &'static str,
    ink: &'static str,
    ink_strong: &'static str,
    accent: &'static str,
    bad: &'static str,
    tone_k: u8,
    head_k: u8,
}

const DARK: Picks = Picks {
    bg: "ansi-0",
    ink: "ansi-7",
    ink_strong: "ansi-15",
    accent: "ansi-12",
    bad: "ansi-9",
    tone_k: 100,
    head_k: 62,
};

const LIGHT: Picks = Picks {
    bg: "ansi-8",
    ink: "ansi-7",
    ink_strong: "ansi-7",
    accent: "ansi-4",
    bad: "ansi-1",
    tone_k: 62,
    head_k: 50,
};

const fn ink_in_bg(pct: u8) -> Value {
    Value::Mix {
        a: "ink",
        pct: Pct::Lit(pct),
        b: "bg",
    }
}

const fn tone(ansi: &'static str) -> Value {
    Value::Mix {
        a: ansi,
        pct: Pct::Var("tone-k"),
        b: "ink",
    }
}

const ROLES: [(&str, Value); 18] = [
    ("bg", Value::Var("bg-src")),
    ("ink", Value::Var("ink-src")),
    ("ink-strong", Value::Var("ink-strong-src")),
    ("muted", ink_in_bg(62)),
    ("faint", ink_in_bg(42)),
    ("line", ink_in_bg(14)),
    ("line-strong", ink_in_bg(30)),
    ("surface", ink_in_bg(5)),
    ("hover", ink_in_bg(9)),
    ("accent", Value::Var("accent-src")),
    (
        "selected",
        Value::Mix {
            a: "accent",
            pct: Pct::Lit(16),
            b: "bg",
        },
    ),
    ("ok", tone("ansi-2")),
    ("warn", tone("ansi-3")),
    ("info", tone("ansi-6")),
    ("bad", Value::Var("bad-src")),
    ("h1", Value::Heading("heading1")),
    ("h2", Value::Heading("heading2")),
    ("h3", Value::Heading("heading3")),
];

fn picks(theme: Theme) -> &'static Picks {
    match theme {
        Theme::Dark => &DARK,
        Theme::Light => &LIGHT,
    }
}

fn render_value(value: &Value) -> String {
    match value {
        Value::Var(name) => format!("var(--r-{name})"),
        Value::Heading(slot) => format!(
            "color-mix(in srgb, var(--r-{slot}, var(--r-ink)) var(--r-head-k), var(--r-ink))"
        ),
        Value::Mix { a, pct, b } => {
            let pct = match pct {
                Pct::Lit(n) => format!("{n}%"),
                Pct::Var(name) => format!("var(--r-{name})"),
            };
            format!("color-mix(in srgb, var(--r-{a}) {pct}, var(--r-{b}))")
        }
    }
}

pub(crate) fn role_lines(theme: Theme, indent: &str) -> String {
    let p = picks(theme);
    let pick_lines = [
        ("bg-src", format!("var(--r-{})", p.bg)),
        ("ink-src", format!("var(--r-{})", p.ink)),
        ("ink-strong-src", format!("var(--r-{})", p.ink_strong)),
        ("accent-src", format!("var(--r-{})", p.accent)),
        ("bad-src", format!("var(--r-{})", p.bad)),
        ("tone-k", format!("{}%", p.tone_k)),
        ("head-k", format!("{}%", p.head_k)),
    ];
    let picks_css = pick_lines
        .iter()
        .map(|(name, value)| format!("{indent}--r-{name}: {value};\n"));
    let roles_css = ROLES
        .iter()
        .map(|(name, value)| format!("{indent}--r-{name}: {};\n", render_value(value)));
    picks_css.chain(roles_css).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::Rgb;

    fn table_for(theme: Theme) -> [Rgb; 16] {
        let web = ralph_server::config::WebSection::default();
        let (name, entries) = match theme {
            Theme::Dark => ("ansi_dark", web.ansi_dark),
            Theme::Light => ("ansi_light", web.ansi_light),
        };
        ralph_server::config::parse_ansi_table(name, &entries)
            .unwrap()
            .map(|[r, g, b]| Rgb { r, g, b })
    }

    const THEMES: [Theme; 2] = [Theme::Dark, Theme::Light];

    fn mix(a: Rgb, pct: u8, b: Rgb) -> Rgb {
        let k = f64::from(pct) / 100.0;
        let ch = |x: u8, y: u8| (f64::from(x) * k + f64::from(y) * (1.0 - k)).round() as u8;
        Rgb {
            r: ch(a.r, b.r),
            g: ch(a.g, b.g),
            b: ch(a.b, b.b),
        }
    }

    fn lum(c: Rgb) -> f64 {
        let lin = |v: u8| {
            let s = f64::from(v) / 255.0;
            if s <= 0.04045 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }

    fn contrast(a: Rgb, b: Rgb) -> f64 {
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    fn pick_target(theme: Theme, name: &str) -> Option<&'static str> {
        let p = picks(theme);
        match name {
            "bg-src" => Some(p.bg),
            "ink-src" => Some(p.ink),
            "ink-strong-src" => Some(p.ink_strong),
            "accent-src" => Some(p.accent),
            "bad-src" => Some(p.bad),
            _ => None,
        }
    }

    fn heading_source(name: &str) -> Option<usize> {
        match name {
            "heading1" => Some(14),
            "heading2" => Some(12),
            "heading3" => Some(13),
            _ => None,
        }
    }

    fn resolve(theme: Theme, ansi: &[Rgb; 16], name: &str, headings_set: bool) -> Rgb {
        if let Some(n) = name.strip_prefix("ansi-") {
            return ansi[n.parse::<usize>().unwrap()];
        }
        if let Some(target) = pick_target(theme, name) {
            return resolve(theme, ansi, target, headings_set);
        }
        let (_, value) = ROLES.iter().find(|(n, _)| *n == name).unwrap();
        match value {
            Value::Var(v) => resolve(theme, ansi, v, headings_set),
            Value::Heading(slot) => {
                let ink = resolve(theme, ansi, "ink", headings_set);
                let head = if headings_set {
                    ansi[heading_source(slot).unwrap()]
                } else {
                    ink
                };
                mix(head, picks(theme).head_k, ink)
            }
            Value::Mix { a, pct, b } => {
                let pct = match pct {
                    Pct::Lit(n) => *n,
                    Pct::Var("head-k") => picks(theme).head_k,
                    Pct::Var(_) => picks(theme).tone_k,
                };
                mix(
                    resolve(theme, ansi, a, headings_set),
                    pct,
                    resolve(theme, ansi, b, headings_set),
                )
            }
        }
    }

    fn role(theme: Theme, name: &str) -> Rgb {
        resolve(theme, &table_for(theme), name, true)
    }

    fn role_without_headings(theme: Theme, name: &str) -> Rgb {
        resolve(theme, &table_for(theme), name, false)
    }

    fn assert_ratio(a: Rgb, b: Rgb, min: f64, what: &str) {
        let c = contrast(a, b);
        assert!(c >= min, "{what}: {c:.2} < {min}");
    }

    #[test]
    fn role_lines_list_picks_then_roles_in_order() {
        let css = role_lines(Theme::Dark, "  ");
        let names: Vec<&str> = css
            .lines()
            .map(|l| l.trim().split(':').next().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "--r-bg-src",
                "--r-ink-src",
                "--r-ink-strong-src",
                "--r-accent-src",
                "--r-bad-src",
                "--r-tone-k",
                "--r-head-k",
                "--r-bg",
                "--r-ink",
                "--r-ink-strong",
                "--r-muted",
                "--r-faint",
                "--r-line",
                "--r-line-strong",
                "--r-surface",
                "--r-hover",
                "--r-accent",
                "--r-selected",
                "--r-ok",
                "--r-warn",
                "--r-info",
                "--r-bad",
                "--r-h1",
                "--r-h2",
                "--r-h3",
            ]
        );
    }

    #[test]
    fn heading_roles_are_emitted_in_every_scope() {
        for (theme, indent) in [
            (Theme::Dark, "  "),
            (Theme::Light, "    "),
            (Theme::Dark, "  "),
            (Theme::Light, "  "),
        ] {
            let css = role_lines(theme, indent);
            for (name, slot) in [("h1", "heading1"), ("h2", "heading2"), ("h3", "heading3")] {
                let line = format!(
                    "{indent}--r-{name}: color-mix(in srgb, var(--r-{slot}, var(--r-ink)) var(--r-head-k), var(--r-ink));\n"
                );
                assert!(css.contains(&line), "{theme:?} lacks {line}");
            }
        }
    }

    #[test]
    fn role_lines_use_the_given_indent() {
        let css = role_lines(Theme::Light, "    ");
        assert!(css.lines().all(|l| l.starts_with("    --r-")));
    }

    #[test]
    fn picks_differ_per_theme() {
        let dark = role_lines(Theme::Dark, "");
        let light = role_lines(Theme::Light, "");
        assert!(dark.contains("--r-bg-src: var(--r-ansi-0);\n"));
        assert!(light.contains("--r-bg-src: var(--r-ansi-8);\n"));
        assert!(dark.contains("--r-ink-strong-src: var(--r-ansi-15);\n"));
        assert!(light.contains("--r-ink-strong-src: var(--r-ansi-7);\n"));
        assert!(dark.contains("--r-accent-src: var(--r-ansi-12);\n"));
        assert!(light.contains("--r-accent-src: var(--r-ansi-4);\n"));
        assert!(dark.contains("--r-bad-src: var(--r-ansi-9);\n"));
        assert!(light.contains("--r-bad-src: var(--r-ansi-1);\n"));
        assert!(dark.contains("--r-tone-k: 100%;\n--r-head-k: 62%;\n"));
        assert!(light.contains("--r-tone-k: 62%;\n--r-head-k: 50%;\n"));
    }

    #[test]
    fn role_formulas_are_exact_in_both_themes() {
        for theme in THEMES {
            let css = role_lines(theme, "");
            for line in [
                "--r-muted: color-mix(in srgb, var(--r-ink) 62%, var(--r-bg));\n",
                "--r-faint: color-mix(in srgb, var(--r-ink) 42%, var(--r-bg));\n",
                "--r-line: color-mix(in srgb, var(--r-ink) 14%, var(--r-bg));\n",
                "--r-line-strong: color-mix(in srgb, var(--r-ink) 30%, var(--r-bg));\n",
                "--r-surface: color-mix(in srgb, var(--r-ink) 5%, var(--r-bg));\n",
                "--r-hover: color-mix(in srgb, var(--r-ink) 9%, var(--r-bg));\n",
                "--r-selected: color-mix(in srgb, var(--r-accent) 16%, var(--r-bg));\n",
                "--r-ok: color-mix(in srgb, var(--r-ansi-2) var(--r-tone-k), var(--r-ink));\n",
                "--r-warn: color-mix(in srgb, var(--r-ansi-3) var(--r-tone-k), var(--r-ink));\n",
                "--r-info: color-mix(in srgb, var(--r-ansi-6) var(--r-tone-k), var(--r-ink));\n",
                "--r-bg: var(--r-bg-src);\n",
                "--r-ink: var(--r-ink-src);\n",
                "--r-ink-strong: var(--r-ink-strong-src);\n",
                "--r-accent: var(--r-accent-src);\n",
                "--r-bad: var(--r-bad-src);\n",
            ] {
                assert!(css.contains(line), "{theme:?} lacks {line}");
            }
        }
    }

    #[test]
    fn evaluator_matches_the_hand_computed_muted_values() {
        assert_eq!(role(Theme::Dark, "muted").to_string(), "#838383");
        assert_eq!(role(Theme::Light, "muted").to_string(), "#636363");
        assert!(
            (contrast(role(Theme::Dark, "muted"), role(Theme::Dark, "bg")) - 5.16).abs() < 0.01
        );
        assert!(
            (contrast(role(Theme::Light, "muted"), role(Theme::Light, "bg")) - 5.37).abs() < 0.01
        );
    }

    #[test]
    fn muted_is_readable_on_bg_and_surface() {
        for theme in THEMES {
            let muted = role(theme, "muted");
            assert_ratio(muted, role(theme, "bg"), 4.5, "muted on bg");
            assert_ratio(muted, role(theme, "surface"), 4.5, "muted on surface");
        }
    }

    #[test]
    fn ink_and_ink_strong_are_readable_on_bg() {
        for theme in THEMES {
            let bg = role(theme, "bg");
            assert_ratio(role(theme, "ink"), bg, 4.5, "ink");
            assert_ratio(role(theme, "ink-strong"), bg, 4.5, "ink-strong");
        }
    }

    #[test]
    fn tones_and_accent_are_readable_on_bg() {
        for theme in THEMES {
            let bg = role(theme, "bg");
            for name in ["ok", "warn", "info", "bad", "accent"] {
                assert_ratio(role(theme, name), bg, 4.5, name);
            }
        }
    }

    #[test]
    fn headings_are_readable_on_bg() {
        let expected = [
            (Theme::Dark, [11.39, 6.91, 4.54]),
            (Theme::Light, [4.99, 8.53, 11.34]),
        ];
        for (theme, ratios) in expected {
            let bg = role(theme, "bg");
            for (name, want) in ["h1", "h2", "h3"].into_iter().zip(ratios) {
                let got = contrast(role(theme, name), bg);
                assert!(got >= 4.5, "{theme:?} {name}: {got:.2}");
                assert!((got - want).abs() < 0.01, "{theme:?} {name}: {got:.2}");
            }
        }
    }

    #[test]
    fn unset_heading_slots_fall_back_to_ink() {
        let expected = [(Theme::Dark, 12.18), (Theme::Light, 17.47)];
        for (theme, want) in expected {
            let bg = role(theme, "bg");
            let ink = role(theme, "ink");
            for name in ["h1", "h2", "h3"] {
                let got = role_without_headings(theme, name);
                assert_eq!(got, ink, "{theme:?} {name}");
                let ratio = contrast(got, bg);
                assert!((ratio - want).abs() < 0.01, "{theme:?} {name}: {ratio:.2}");
            }
        }
    }

    #[test]
    fn mix_rounds_to_nearest() {
        let a = Rgb { r: 255, g: 0, b: 1 };
        let b = Rgb { r: 0, g: 0, b: 0 };
        let m = mix(a, 50, b);
        assert_eq!((m.r, m.g, m.b), (128, 0, 1));
    }

    #[test]
    fn contrast_of_black_on_white_is_21() {
        let white = Rgb {
            r: 255,
            g: 255,
            b: 255,
        };
        let black = Rgb { r: 0, g: 0, b: 0 };
        assert!((contrast(white, black) - 21.0).abs() < 1e-9);
    }
}
