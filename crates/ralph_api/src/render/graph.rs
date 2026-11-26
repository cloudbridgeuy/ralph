use crate::threads::cells::Cell;

const CELL_W: f64 = 8.0;
const ROW_H: f64 = 24.0;
const MID: f64 = ROW_H / 2.0;
const OVERSHOOT: f64 = 1.0;
const RADIUS: f64 = 3.25;
const CORNER: f64 = 4.0;
const MARK: f64 = 3.0;
const ARROW_HALF_H: f64 = 3.5;
const ARROW_TIP: f64 = 0.5;
const ARROW_BASE: f64 = 6.5;

fn line(tone: &str, horizontal: bool, d: &str) -> String {
    let h = if horizontal { " h" } else { "" };
    format!("<path class=\"ln {tone}{h}\" d=\"{d}\"/>")
}

fn node(tone: &str, full: bool, cx: f64) -> String {
    let full = if full { " full" } else { "" };
    format!("<circle class=\"nd {tone}{full}\" cx=\"{cx}\" cy=\"{MID}\" r=\"{RADIUS}\"/>")
}

fn trunk(cx: f64) -> String {
    format!("M{cx} {}V{}", -OVERSHOOT, ROW_H + OVERSHOOT)
}

fn shapes(column: usize, cell: &Cell) -> String {
    let tone = cell.tone.name();
    let x = column as f64 * CELL_W;
    let cx = x + CELL_W / 2.0;
    let right = x + CELL_W;
    match cell.ch {
        '●' if column == 0 => {
            line(tone, false, &format!("M{cx} {MID}V{}", ROW_H + OVERSHOOT)) + &node(tone, true, cx)
        }
        '●' => node(tone, true, cx),
        '○' => node(tone, false, cx),
        '├' => line(tone, false, &trunk(cx)) + &line(tone, true, &format!("M{cx} {MID}H{right}")),
        '╰' => line(
            tone,
            false,
            &format!(
                "M{cx} {}V{}Q{cx} {MID} {} {MID}",
                -OVERSHOOT,
                MID - CORNER,
                cx + CORNER
            ),
        ),
        '─' => line(tone, true, &format!("M{x} {MID}H{right}")),
        '◄' => {
            line(tone, true, &format!("M{cx} {MID}H{right}"))
                + &format!(
                    "<path class=\"arrow {tone}\" d=\"M{} {MID}L{} {}V{}Z\"/>",
                    x + ARROW_TIP,
                    x + ARROW_BASE,
                    MID - ARROW_HALF_H,
                    MID + ARROW_HALF_H
                )
        }
        '✕' => line(
            tone,
            false,
            &format!(
                "M{} {}L{} {}M{} {}L{} {}",
                cx - MARK,
                MID - MARK,
                cx + MARK,
                MID + MARK,
                cx + MARK,
                MID - MARK,
                cx - MARK,
                MID + MARK
            ),
        ),
        _ => String::new(),
    }
}

pub fn graph_svg(cells: &[Cell]) -> String {
    let width = cells.len() as f64 * CELL_W;
    let body: String = cells
        .iter()
        .enumerate()
        .map(|(column, cell)| shapes(column, cell))
        .collect();
    format!(
        "<r-graph><svg width=\"{width}\" height=\"{ROW_H}\" viewBox=\"0 0 {width} {ROW_H}\" aria-hidden=\"true\">{body}</svg></r-graph>"
    )
}

#[cfg(test)]
mod tests {
    use super::graph_svg;
    use crate::render::golden::assert_golden;
    use crate::threads::cells::{lane_cells, Cell, Lane, Tone};

    const OPEN: &str =
        "<r-graph><svg width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" aria-hidden=\"true\">";
    const CLOSE: &str = "</svg></r-graph>";
    const ALL: [Lane; 5] = [
        Lane::Head,
        Lane::Open,
        Lane::Parked,
        Lane::Merged,
        Lane::Dropped,
    ];

    fn svg(lane: Lane, last: bool) -> String {
        graph_svg(&lane_cells(lane, last))
    }

    fn inner(lane: Lane, last: bool) -> String {
        let svg = svg(lane, last);
        svg.strip_prefix(OPEN)
            .and_then(|rest| rest.strip_suffix(CLOSE))
            .unwrap()
            .to_string()
    }

    #[test]
    fn graph_svg_golden() {
        let rows: String = ALL
            .iter()
            .enumerate()
            .map(|(index, lane)| svg(*lane, index + 1 == ALL.len()))
            .collect();
        assert_golden("graph", &rows);
    }

    #[test]
    fn each_kind_matches_the_mockup_paths() {
        assert_eq!(
            inner(Lane::Head, false),
            concat!(
                "<path class=\"ln plain\" d=\"M4 12V25\"/>",
                "<circle class=\"nd plain full\" cx=\"4\" cy=\"12\" r=\"3.25\"/>"
            )
        );
        assert_eq!(
            inner(Lane::Open, false),
            concat!(
                "<path class=\"ln plain\" d=\"M4 -1V25\"/>",
                "<path class=\"ln plain h\" d=\"M4 12H8\"/>",
                "<path class=\"ln plain h\" d=\"M8 12H16\"/>",
                "<circle class=\"nd plain\" cx=\"20\" cy=\"12\" r=\"3.25\"/>"
            )
        );
        assert_eq!(
            inner(Lane::Parked, false),
            concat!(
                "<path class=\"ln plain\" d=\"M4 -1V25\"/>",
                "<path class=\"ln plain h\" d=\"M4 12H8\"/>",
                "<path class=\"ln warn h\" d=\"M8 12H16\"/>",
                "<circle class=\"nd warn\" cx=\"20\" cy=\"12\" r=\"3.25\"/>"
            )
        );
        assert_eq!(
            inner(Lane::Merged, false),
            concat!(
                "<path class=\"ln plain\" d=\"M4 -1V25\"/>",
                "<path class=\"ln plain h\" d=\"M4 12H8\"/>",
                "<path class=\"ln ok h\" d=\"M12 12H16\"/>",
                "<path class=\"arrow ok\" d=\"M8.5 12L14.5 8.5V15.5Z\"/>",
                "<circle class=\"nd ok full\" cx=\"20\" cy=\"12\" r=\"3.25\"/>"
            )
        );
        assert_eq!(
            inner(Lane::Dropped, false),
            concat!(
                "<path class=\"ln plain\" d=\"M4 -1V25\"/>",
                "<path class=\"ln plain h\" d=\"M4 12H8\"/>",
                "<path class=\"ln bad h\" d=\"M8 12H16\"/>",
                "<path class=\"ln bad\" d=\"M17 9L23 15M23 9L17 15\"/>"
            )
        );
    }

    #[test]
    fn the_last_row_stops_the_trunk_at_the_curve() {
        assert_eq!(
            inner(Lane::Dropped, true),
            concat!(
                "<path class=\"ln plain\" d=\"M4 -1V8Q4 12 8 12\"/>",
                "<path class=\"ln bad h\" d=\"M8 12H16\"/>",
                "<path class=\"ln bad\" d=\"M17 9L23 15M23 9L17 15\"/>"
            )
        );
        assert_eq!(
            inner(Lane::Open, true),
            concat!(
                "<path class=\"ln plain\" d=\"M4 -1V8Q4 12 8 12\"/>",
                "<path class=\"ln plain h\" d=\"M8 12H16\"/>",
                "<circle class=\"nd plain\" cx=\"20\" cy=\"12\" r=\"3.25\"/>"
            )
        );
    }

    #[test]
    fn every_row_is_one_svg_in_one_graph_element() {
        for lane in ALL {
            let svg = svg(lane, false);
            assert!(svg.starts_with(OPEN), "{lane:?}");
            assert!(svg.ends_with(CLOSE), "{lane:?}");
            assert_eq!(svg.matches("<svg").count(), 1);
        }
    }

    #[test]
    fn no_colour_literal_or_inline_style_appears() {
        for lane in ALL {
            let svg = svg(lane, true);
            for literal in ["#", "rgb", "hsl", "style=", "stroke=", "fill=", "url("] {
                assert!(!svg.contains(literal), "{lane:?} {literal}");
            }
        }
    }

    #[test]
    fn blank_and_unknown_glyphs_draw_nothing() {
        let cells = [
            Cell {
                ch: ' ',
                tone: Tone::Plain,
            },
            Cell {
                ch: '?',
                tone: Tone::Bad,
            },
        ];
        assert_eq!(
            graph_svg(&cells),
            "<r-graph><svg width=\"16\" height=\"24\" viewBox=\"0 0 16 24\" aria-hidden=\"true\"></svg></r-graph>"
        );
    }

    #[test]
    fn a_node_in_the_trunk_column_runs_the_trunk_down_only_there() {
        let at = |column: usize| {
            let mut cells = vec![
                Cell {
                    ch: ' ',
                    tone: Tone::Plain
                };
                3
            ];
            cells[column] = Cell {
                ch: '●',
                tone: Tone::Ok,
            };
            graph_svg(&cells)
        };
        assert!(at(0).contains("M4 12V25"));
        assert!(!at(2).contains('V'));
    }
}
