use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::ansi::{truecolor_background, truecolor_foreground, CLEAR_EOL, DIM, RESET};
use crate::chat_status::truncate_to_width;

const RULE: &str = "─";
const LEAD: &str = "── ";
pub const MARGIN: &str = "  ";
const TAB: &str = "    ";
const CSI: &str = "\x1b[";
const KITTY_OPEN: &str = "\x1b_G";
const FILL_STEP: Percent = Percent(8);
const RULE_STEP: Percent = Percent(40);

#[derive(Debug, Clone, Copy)]
pub struct Columns(pub u16);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Debug, Clone, Copy)]
pub struct ThemeTones {
    pub background: Option<Rgb>,
    pub foreground: Option<Rgb>,
}

#[derive(Debug, Clone, Copy)]
pub enum Backdrop {
    Painted,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Shaded { fill: Rgb, rule: Rgb },
    Open,
}

#[derive(Debug, Clone, Copy)]
pub struct Percent(pub u8);

#[derive(Debug, Clone, Copy)]
enum Tail {
    Erase,
    Filled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    Frame,
    Flow,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Spacing {
    line_ends: Option<usize>,
}

impl Spacing {
    pub fn append(&mut self, out: &mut String, block: &str, kind: BlockKind) {
        if kind == BlockKind::Frame {
            self.push(out, self.gap_above_frame());
        }
        self.push(out, block);
    }

    fn gap_above_frame(self) -> &'static str {
        match self.line_ends {
            Some(0) => "\n\n",
            Some(1) => "\n",
            None | Some(_) => "",
        }
    }

    fn push(&mut self, out: &mut String, text: &str) {
        if text.is_empty() {
            return;
        }
        let body = text.trim_end_matches('\n');
        let carried = self.line_ends.filter(|_| body.is_empty()).unwrap_or(0);
        self.line_ends = Some(carried + text.len() - body.len());
        out.push_str(text);
    }
}

pub fn mix(base: Rgb, toward: Rgb, step: Percent) -> Rgb {
    let step = u32::from(step.0.min(100));
    let channel = |from: u8, to: u8| {
        let mixed = (u32::from(from) * (100 - step) + u32::from(to) * step + 50) / 100;
        u8::try_from(mixed).unwrap_or(u8::MAX)
    };
    Rgb {
        r: channel(base.r, toward.r),
        g: channel(base.g, toward.g),
        b: channel(base.b, toward.b),
    }
}

pub fn surface(tones: ThemeTones, backdrop: Backdrop) -> Surface {
    match (backdrop, tones.background, tones.foreground) {
        (Backdrop::Painted, Some(background), Some(foreground)) => Surface::Shaded {
            fill: mix(background, foreground, FILL_STEP),
            rule: mix(background, foreground, RULE_STEP),
        },
        _ => Surface::Open,
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Label<'a> {
    Bare,
    Language(&'a str),
    File {
        path: &'a str,
        kind: &'a str,
        ink: &'a str,
    },
}

#[derive(Debug, Clone, Copy)]
enum Keep {
    Start,
    End,
}

pub fn rule_line(label: &Label<'_>, cols: Columns) -> String {
    let cols = usize::from(cols.0);
    let head = match label {
        Label::Bare => String::new(),
        Label::Language(name) => heading(name, cols, Keep::Start),
        Label::File { path, kind, .. } => heading(&format!("{path} · {kind}"), cols, Keep::End),
    };
    let rest = cols.saturating_sub(head.width());
    head + &RULE.repeat(rest)
}

fn heading(title: &str, cols: usize, keep: Keep) -> String {
    let room = cols.saturating_sub(LEAD.width() + 1);
    if room == 0 {
        String::new()
    } else if title.width() <= room {
        format!("{LEAD}{title} ")
    } else {
        match keep {
            Keep::Start => format!("{LEAD}{}… ", truncate_to_width(title, room - 1)),
            Keep::End => format!("{LEAD}…{} ", end_to_width(title, room - 1)),
        }
    }
}

fn end_to_width(text: &str, width: usize) -> &str {
    let mut used = 0;
    let start = text
        .char_indices()
        .rev()
        .take_while(|(_, c)| {
            used += c.width().unwrap_or(0);
            used <= width
        })
        .last()
        .map_or(text.len(), |(at, _)| at);
    &text[start..]
}

pub fn frame_block(label: &Label<'_>, body: &str, surface: Surface, cols: Columns) -> String {
    let open = rule_line(label, cols);
    let ink = match label {
        Label::File { ink, .. } => Some(*ink),
        Label::Bare | Label::Language(_) => None,
    };
    match surface {
        Surface::Open => {
            let rows = body_rows(body, open_row);
            let close = rule_line(&Label::Bare, cols);
            let ink = ink.unwrap_or(DIM);
            format!("{ink}{open}{RESET}\n{rows}{DIM}{close}{RESET}\n")
        }
        Surface::Shaded { fill, rule } => {
            let rows = body_rows(body, |content| shaded_row(content, fill, cols));
            let pad = shaded_row("", fill, cols);
            let rule = truecolor_foreground(rule.r, rule.g, rule.b);
            let ink = ink.unwrap_or(&rule);
            format!("{ink}{open}{RESET}\n{pad}{rows}{pad}")
        }
    }
}

fn body_rows(body: &str, row: impl Fn(&str) -> String) -> String {
    body.split('\n')
        .map(|line| {
            if line.contains(KITTY_OPEN) {
                format!("{line}\n")
            } else {
                row(&line.replace('\t', TAB))
            }
        })
        .collect()
}

fn open_row(content: &str) -> String {
    format!("{MARGIN}{content}\n")
}

fn shaded_row(content: &str, fill: Rgb, cols: Columns) -> String {
    let background = truecolor_background(fill.r, fill.g, fill.b);
    let row = format!("{MARGIN}{content}");
    let tail = match row_tail(&row, cols) {
        Tail::Erase => CLEAR_EOL,
        Tail::Filled => "",
    };
    let row = reassert_fill(&row, &background);
    format!("{background}{row}{tail}{RESET}\n")
}

fn csi_pieces(row: &str) -> impl Iterator<Item = (&str, &str)> {
    row.split(CSI).enumerate().map(|(index, piece)| {
        let end = if index == 0 {
            0
        } else {
            piece
                .find(|c| ('@'..='~').contains(&c))
                .map_or(piece.len(), |at| at + 1)
        };
        piece.split_at(end)
    })
}

fn row_tail(row: &str, cols: Columns) -> Tail {
    let cols = usize::from(cols.0);
    let end = csi_pieces(row)
        .flat_map(|(_, text)| text.chars())
        .filter_map(UnicodeWidthChar::width)
        .fold(0, |column, width| {
            if column + width > cols {
                width
            } else {
                column + width
            }
        });
    if cols > 0 && end == cols {
        Tail::Filled
    } else {
        Tail::Erase
    }
}

fn reassert_fill(row: &str, background: &str) -> String {
    csi_pieces(row)
        .enumerate()
        .map(|(index, (sequence, text))| {
            let opener = if index == 0 { "" } else { CSI };
            let again = match sequence.strip_suffix('m') {
                Some(codes) if drops_background(codes) => background,
                _ => "",
            };
            format!("{opener}{sequence}{again}{text}")
        })
        .collect()
}

fn drops_background(codes: &str) -> bool {
    let mut codes = codes.split(';').map(|code| code.parse().unwrap_or(0_u16));
    let mut dropped = false;
    while let Some(code) = codes.next() {
        match code {
            0 | 49 => dropped = true,
            40..=47 | 100..=107 => dropped = false,
            38 | 48 => {
                match codes.next() {
                    Some(5) => {
                        codes.next();
                    }
                    Some(2) => {
                        codes.nth(2);
                    }
                    _ => {}
                }
                dropped &= code != 48;
            }
            _ => {}
        }
    }
    dropped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ansi::{truecolor_background, truecolor_foreground, CLEAR_EOL};
    use unicode_width::UnicodeWidthStr;

    const LONG: &str = "averyveryverylonglanguagename";

    #[test]
    fn rule_line_carries_language() {
        let line = rule_line(&Label::Language("rust"), Columns(20));
        assert_eq!(line, format!("── rust {}", "─".repeat(12)));
    }

    #[test]
    fn rule_line_bare_is_solid() {
        assert_eq!(rule_line(&Label::Bare, Columns(5)), "─────");
    }

    #[test]
    fn rule_line_truncates_long_label() {
        for cols in 0..40 {
            let line = rule_line(&Label::Language(LONG), Columns(cols));
            assert_eq!(line.width(), usize::from(cols), "{line:?}");
        }
        let line = rule_line(&Label::Language(LONG), Columns(12));
        assert_eq!(line, "── averyve… ");
    }

    fn file(path: &str) -> Label<'_> {
        Label::File {
            path,
            kind: "diff",
            ink: "<ink>",
        }
    }

    #[test]
    fn rule_line_carries_file_and_kind() {
        let line = rule_line(&file("src/a.rs"), Columns(24));
        assert_eq!(line, "── src/a.rs · diff ─────");
    }

    #[test]
    fn rule_line_keeps_the_end_of_a_long_path() {
        let line = rule_line(&file("some/very/long/path/a.rs"), Columns(20));
        assert_eq!(line, "── …ath/a.rs · diff ");
        for cols in 0..40 {
            let line = rule_line(&file("some/very/日本/path/a.rs"), Columns(cols));
            assert_eq!(line.width(), usize::from(cols), "{line:?}");
        }
    }

    #[test]
    fn file_rule_uses_its_own_ink_in_both_looks() {
        for surface in [SHADED, Surface::Open] {
            let out = frame_block(&file("a"), "x", surface, Columns(12));
            assert!(out.starts_with("<ink>── a · diff"), "{out:?}");
        }
    }

    #[test]
    fn frame_indents_code_between_rules() {
        let out = frame_block(&Label::Language("sh"), "a\n\nb", Surface::Open, Columns(8));
        assert_eq!(
            out,
            "\x1b[2m── sh ──\x1b[0m\n  a\n  \n  b\n\x1b[2m────────\x1b[0m\n"
        );
    }

    #[test]
    fn apc_row_gets_no_margin() {
        let apc = "\x1b_Ga=T;QQ==\x1b\\";
        let body = format!("{apc}\ntail");
        let out = frame_block(&Label::Bare, &body, Surface::Open, Columns(4));
        assert!(out.contains(&format!("\n{apc}\n  tail\n")), "{out:?}");
    }

    const DARK: ThemeTones = ThemeTones {
        background: Some(Rgb {
            r: 43,
            g: 48,
            b: 59,
        }),
        foreground: Some(Rgb {
            r: 192,
            g: 197,
            b: 206,
        }),
    };
    const FILL: Rgb = Rgb {
        r: 55,
        g: 60,
        b: 71,
    };
    const SHADED: Surface = Surface::Shaded {
        fill: FILL,
        rule: Rgb {
            r: 103,
            g: 108,
            b: 118,
        },
    };

    fn fill() -> String {
        truecolor_background(FILL.r, FILL.g, FILL.b)
    }

    fn shaded(body: &str, cols: u16) -> Vec<String> {
        frame_block(&Label::Bare, body, SHADED, Columns(cols))
            .lines()
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn surface_dark_theme_lightens() {
        assert_eq!(surface(DARK, Backdrop::Painted), SHADED);
    }

    #[test]
    fn surface_light_theme_darkens() {
        let tones = ThemeTones {
            background: Some(Rgb {
                r: 255,
                g: 255,
                b: 255,
            }),
            foreground: Some(Rgb {
                r: 55,
                g: 55,
                b: 55,
            }),
        };
        let Surface::Shaded { fill, rule } = surface(tones, Backdrop::Painted) else {
            panic!("light theme must be shaded");
        };
        assert_eq!(
            fill,
            Rgb {
                r: 239,
                g: 239,
                b: 239
            }
        );
        assert_eq!(
            rule,
            Rgb {
                r: 175,
                g: 175,
                b: 175
            }
        );
    }

    #[test]
    fn surface_disabled_or_missing_tone_is_open() {
        assert_eq!(surface(DARK, Backdrop::Disabled), Surface::Open);
        for tones in [
            ThemeTones {
                background: None,
                ..DARK
            },
            ThemeTones {
                foreground: None,
                ..DARK
            },
        ] {
            assert_eq!(surface(tones, Backdrop::Painted), Surface::Open);
        }
    }

    #[test]
    fn shaded_body_has_pad_rows_and_no_closing_rule() {
        let rows = shaded("a", 10);
        let pad = format!("{}  {CLEAR_EOL}{RESET}", fill());
        assert_eq!(rows.len(), 4, "{rows:?}");
        assert!(rows[0].contains(&"─".repeat(10)), "{rows:?}");
        assert_eq!(rows[1], pad);
        assert_eq!(rows[2], format!("{}  a{CLEAR_EOL}{RESET}", fill()));
        assert_eq!(rows[3], pad);
    }

    #[test]
    fn fill_survives_a_reset_in_code() {
        let body = format!("a{RESET}b\x1b[mc\x1b[49md");
        let row = &shaded(&body, 20)[2];
        let fill = fill();
        assert!(
            row.contains(&format!("a{RESET}{fill}b\x1b[m{fill}c\x1b[49m{fill}d")),
            "{row:?}"
        );
    }

    #[test]
    fn extended_colour_arguments_are_not_a_reset() {
        let colour = truecolor_foreground(0, 0, 0);
        let own = truecolor_background(0, 49, 0);
        let body = format!("{colour}a{own}b");
        let row = &shaded(&body, 20)[2];
        assert!(row.contains(&format!("{colour}a{own}b")), "{row:?}");
    }

    #[test]
    fn exact_width_row_skips_the_erase() {
        for body in ["abcd", "abcdefghij", "abcd\x1b[0m"] {
            let row = &shaded(body, 6)[2];
            assert!(!row.contains(CLEAR_EOL), "{body:?} {row:?}");
        }
        for body in ["abc", "abcde", "abc日"] {
            let row = &shaded(body, 6)[2];
            assert!(row.contains(CLEAR_EOL), "{body:?} {row:?}");
        }
    }

    #[test]
    fn tabs_expand_before_the_fill() {
        let row = &shaded("\tx\ty", 20)[2];
        assert!(row.contains("      x    y"), "{row:?}");
        let open = frame_block(&Label::Bare, "\tx", Surface::Open, Columns(8));
        assert!(open.contains("\n      x\n"), "{open:?}");
    }

    #[test]
    fn apc_row_passes_unshaded() {
        let apc = "\x1b_Ga=T;QQ==\x1b\\";
        assert_eq!(shaded(apc, 10)[2], apc);
    }

    #[test]
    fn no_row_leaks_the_background() {
        let rows = shaded("a\n\nb", 10);
        assert!(!rows[0].contains(&fill()), "{rows:?}");
        for row in &rows {
            assert!(row.ends_with(RESET), "{row:?}");
        }
    }

    fn spaced(blocks: &[(&str, BlockKind)]) -> String {
        let mut spacing = Spacing::default();
        let mut out = String::new();
        for (block, kind) in blocks {
            spacing.append(&mut out, block, *kind);
        }
        out
    }

    #[test]
    fn a_frame_gets_a_blank_row_above() {
        use BlockKind::{Flow, Frame};
        for above in ["a", "a\n", "a\n\n"] {
            assert_eq!(
                spaced(&[(above, Flow), ("F\n", Frame)]),
                "a\n\nF\n",
                "{above:?}"
            );
        }
        assert_eq!(spaced(&[("F\n\n", Frame), ("G\n", Frame)]), "F\n\nG\n");
        assert_eq!(
            spaced(&[("a\n", Flow), ("\n", Flow), ("F\n", Frame)]),
            "a\n\nF\n"
        );
    }

    #[test]
    fn nothing_is_added_above_the_first_block_or_flowing_text() {
        use BlockKind::{Flow, Frame};
        assert_eq!(spaced(&[("F\n", Frame)]), "F\n");
        assert_eq!(spaced(&[("", Flow), ("F\n", Frame)]), "F\n");
        assert_eq!(spaced(&[("a", Flow), ("b\n", Flow)]), "ab\n");
        assert_eq!(
            spaced(&[("a\n", Flow), ("", Flow), ("F\n", Frame)]),
            "a\n\nF\n"
        );
    }
}
