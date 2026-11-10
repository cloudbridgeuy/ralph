use super::wrap::render_prose;
use crate::markdown::{markdown_skin, Palette};
use unicode_width::UnicodeWidthStr;

const CELLS: [[&str; 5]; 3] = [
    [
        "Component name here",
        "Owner of the area",
        "Current status text",
        "Next planned action",
        "Risk and caveats",
    ],
    [
        "alpha beta gamma delta",
        "epsilon zeta eta theta",
        "iota kappa lambda mu",
        "nu xi omicron pi rho",
        "sigma tau upsilon phi",
    ],
    [
        "one two three four five",
        "six seven eight nine ten",
        "red green blue yellow",
        "north south east west",
        "spring summer autumn winter",
    ],
];

fn table() -> String {
    let row = |cells: &[&str; 5]| format!("| {} |\n", cells.join(" | "));
    let mut text = row(&CELLS[0]);
    text.push_str("|---|---|---|---|---|\n");
    CELLS[1..]
        .iter()
        .for_each(|cells| text.push_str(&row(cells)));
    text
}

fn strip_ansi(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            chars.by_ref().find(|ch| ch.is_ascii_alphabetic());
        } else {
            out.push(ch);
        }
    }
    out
}

fn lines_at(width: usize) -> Vec<String> {
    let skin = markdown_skin(&Palette::default());
    let rendered = render_prose(&skin, &table(), width);
    strip_ansi(&rendered).lines().map(str::to_string).collect()
}

fn intact(lines: &[String], cell: &str) -> bool {
    lines.iter().any(|line| line.contains(cell))
}

#[test]
fn every_table_line_fits_the_width() {
    for width in [80, 160] {
        for line in lines_at(width) {
            assert!(line.width() <= width, "{width} columns: {line}");
        }
    }
}

#[test]
fn a_wide_terminal_keeps_every_cell_on_one_line() {
    let lines = lines_at(160);
    for cell in CELLS.iter().flatten() {
        assert!(intact(&lines, cell), "{cell} wrapped at 160\n{lines:#?}");
    }
}

#[test]
fn a_narrow_terminal_wraps_at_least_one_cell() {
    let lines = lines_at(80);
    let wrapped = CELLS.iter().flatten().any(|cell| !intact(&lines, cell));
    assert!(wrapped, "no cell wrapped at 80\n{lines:#?}");
}
