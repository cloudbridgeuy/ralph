use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub fn from_hex(text: &str) -> Option<Rgb> {
        let digits = text.strip_prefix('#')?;
        if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let channel = |range: std::ops::Range<usize>| u8::from_str_radix(&digits[range], 16).ok();
        Some(Rgb {
            r: channel(0..2)?,
            g: channel(2..4)?,
            b: channel(4..6)?,
        })
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSpec {
    Ansi(u8),
    Rgb(Rgb),
}

pub struct Slot {
    pub name: String,
    pub color: ColorSpec,
}

pub struct TokensInput<'a> {
    pub font: &'a str,
    pub ansi_dark: &'a [Rgb; 16],
    pub ansi_light: &'a [Rgb; 16],
    pub slots: &'a [Slot],
}

const MAX_ANSI: u8 = 15;

fn sanitize_font(font: &str) -> String {
    font.chars()
        .filter(|c| !c.is_control() && !matches!(c, ';' | '{' | '}' | '<' | '>' | '\\'))
        .collect::<String>()
        .trim()
        .to_string()
}

fn slot_line(slot: &Slot) -> Option<String> {
    match slot.color {
        ColorSpec::Ansi(n) if n <= MAX_ANSI => {
            Some(format!("  --r-{}: var(--r-ansi-{n});\n", slot.name))
        }
        ColorSpec::Ansi(_) => None,
        ColorSpec::Rgb(rgb) => Some(format!("  --r-{}: {rgb};\n", slot.name)),
    }
}

fn ansi_lines(table: &[Rgb; 16], indent: &str) -> String {
    table
        .iter()
        .enumerate()
        .map(|(i, rgb)| format!("{indent}--r-ansi-{i}: {rgb};\n"))
        .collect()
}

pub fn tokens_css(input: &TokensInput) -> String {
    let mut css = String::from(":root {\n");
    css.push_str(&format!("  --r-font: {};\n", sanitize_font(input.font)));
    css.push_str(&ansi_lines(input.ansi_dark, "  "));
    css.extend(input.slots.iter().filter_map(slot_line));
    css.push_str("}\n@media (prefers-color-scheme: light) {\n  :root {\n");
    css.push_str(&ansi_lines(input.ansi_light, "    "));
    css.push_str("  }\n}\n");
    css
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(base: u8) -> [Rgb; 16] {
        std::array::from_fn(|i| {
            let v = base + i as u8;
            Rgb { r: v, g: v, b: v }
        })
    }

    fn slot(name: &str, color: ColorSpec) -> Slot {
        Slot {
            name: name.to_string(),
            color,
        }
    }

    fn render(font: &str, slots: &[Slot]) -> String {
        let dark = table(0);
        let light = table(100);
        tokens_css(&TokensInput {
            font,
            ansi_dark: &dark,
            ansi_light: &light,
            slots,
        })
    }

    #[test]
    fn from_hex_accepts_lower_and_upper() {
        assert_eq!(
            Rgb::from_hex("#0c0c0c"),
            Some(Rgb {
                r: 12,
                g: 12,
                b: 12
            })
        );
        assert_eq!(
            Rgb::from_hex("#AABBCC"),
            Some(Rgb {
                r: 0xaa,
                g: 0xbb,
                b: 0xcc
            })
        );
    }

    #[test]
    fn from_hex_rejects_bad_input() {
        for text in ["0c0c0c", "#fff", "#12345g", "#1234567", ""] {
            assert_eq!(Rgb::from_hex(text), None, "{text}");
        }
    }

    #[test]
    fn display_round_trips_lowercase() {
        let rgb = Rgb::from_hex("#AABB0C").unwrap();
        assert_eq!(rgb.to_string(), "#aabb0c");
        assert_eq!(Rgb::from_hex(&rgb.to_string()), Some(rgb));
    }

    #[test]
    fn golden_output() {
        let dark = [Rgb { r: 1, g: 2, b: 3 }; 16];
        let light = [Rgb {
            r: 255,
            g: 254,
            b: 253,
        }; 16];
        let slots = [
            slot("bg", ColorSpec::Rgb(Rgb { r: 0, g: 0, b: 0 })),
            slot("accent", ColorSpec::Ansi(3)),
        ];
        let css = tokens_css(&TokensInput {
            font: "monospace",
            ansi_dark: &dark,
            ansi_light: &light,
            slots: &slots,
        });
        let mut expected = String::from(":root {\n  --r-font: monospace;\n");
        for i in 0..16 {
            expected.push_str(&format!("  --r-ansi-{i}: #010203;\n"));
        }
        expected.push_str("  --r-bg: #000000;\n  --r-accent: var(--r-ansi-3);\n}\n");
        expected.push_str("@media (prefers-color-scheme: light) {\n  :root {\n");
        for i in 0..16 {
            expected.push_str(&format!("    --r-ansi-{i}: #fffefd;\n"));
        }
        expected.push_str("  }\n}\n");
        assert_eq!(css, expected);
        assert!(css.starts_with(":root {\n  --r-font: monospace;\n  --r-ansi-0: #010203;\n"));
        assert!(css.ends_with("    --r-ansi-15: #fffefd;\n  }\n}\n"));
    }

    #[test]
    fn ansi_vars_in_order() {
        let css = render("x", &[]);
        let dark_part = css.split("@media").next().unwrap();
        let positions: Vec<usize> = (0..16)
            .map(|i| dark_part.find(&format!("--r-ansi-{i}:")).unwrap())
            .collect();
        assert!(positions.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn slot_order_preserved() {
        let css = render(
            "x",
            &[
                slot("zeta", ColorSpec::Ansi(1)),
                slot("alpha", ColorSpec::Ansi(2)),
            ],
        );
        assert!(css.find("--r-zeta:").unwrap() < css.find("--r-alpha:").unwrap());
    }

    #[test]
    fn ansi_slot_uses_var() {
        let css = render("x", &[slot("fg", ColorSpec::Ansi(3))]);
        assert!(css.contains("  --r-fg: var(--r-ansi-3);\n"));
    }

    #[test]
    fn ansi_out_of_range_emits_nothing() {
        let css = render("x", &[slot("bad", ColorSpec::Ansi(16))]);
        assert!(!css.contains("--r-bad"));
        assert!(!css.contains("--r-ansi-16"));
        assert_eq!(css, render("x", &[]));
    }

    #[test]
    fn light_block_has_only_light_ansi_vars() {
        let css = render("x", &[slot("fg", ColorSpec::Ansi(3))]);
        let light = css.split("@media").nth(1).unwrap();
        assert!(!light.contains("--r-font"));
        assert!(!light.contains("--r-fg"));
        assert_eq!(light.matches("--r-").count(), 16);
        assert!(light.contains("--r-ansi-0: #646464;"));
        assert!(light.contains("--r-ansi-15: #737373;"));
    }

    #[test]
    fn deterministic() {
        let slots = [slot("fg", ColorSpec::Ansi(3))];
        assert_eq!(render("x", &slots), render("x", &slots));
    }

    #[test]
    fn font_cannot_break_out() {
        let css = render("a; } body { x: y", &[]);
        let line = css.lines().find(|l| l.contains("--r-font")).unwrap();
        assert_eq!(line, "  --r-font: a  body  x: y;");
        assert_eq!(line.matches(';').count(), 1);
        assert_eq!(css.matches('{').count(), 3);
        assert_eq!(css.matches('}').count(), 3);
    }

    #[test]
    fn font_drops_control_and_angle_chars() {
        let css = render("a\n\r\t<b>\\c\u{7}", &[]);
        assert!(css.contains("  --r-font: abc;\n"));
    }

    #[test]
    fn font_is_trimmed() {
        let css = render("  Fira Code  ", &[]);
        assert!(css.contains("  --r-font: Fira Code;\n"));
    }
}
