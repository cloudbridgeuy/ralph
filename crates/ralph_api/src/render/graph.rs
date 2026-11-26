use ralph_core::chat_thread_graph::{GraphRow, LaneEnd, RowKind, Tone};

const CELL_W: usize = 12;
const ROW_H: usize = 24;
const HALF_W: usize = CELL_W / 2;
const HALF_H: usize = ROW_H / 2;
const RADIUS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Arms {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Glyph {
    Dot,
    Ring,
    Cross,
    Arrow,
    Arms(Arms),
}

const fn arms(up: bool, down: bool, left: bool, right: bool) -> Glyph {
    Glyph::Arms(Arms {
        up,
        down,
        left,
        right,
    })
}

fn glyph(ch: char) -> Option<Glyph> {
    match ch {
        '●' => Some(Glyph::Dot),
        '○' => Some(Glyph::Ring),
        '✕' => Some(Glyph::Cross),
        '◄' => Some(Glyph::Arrow),
        '│' => Some(arms(true, true, false, false)),
        '─' => Some(arms(false, false, true, true)),
        '├' => Some(arms(true, true, false, true)),
        '┤' => Some(arms(true, true, true, false)),
        '┴' => Some(arms(true, false, true, true)),
        '┬' => Some(arms(false, true, true, true)),
        '┼' => Some(arms(true, true, true, true)),
        '╭' => Some(arms(false, true, false, true)),
        '╮' => Some(arms(false, true, true, false)),
        '╰' => Some(arms(true, false, false, true)),
        '╯' => Some(arms(true, false, true, false)),
        _ => None,
    }
}

fn tone_class(tone: Tone) -> &'static str {
    match tone {
        Tone::Plain => "plain",
        Tone::Green => "green",
        Tone::Yellow => "yellow",
        Tone::Red => "red",
    }
}

fn kind_name(kind: &RowKind) -> &'static str {
    match kind {
        RowKind::Trunk { .. } => "head",
        RowKind::Tip {
            end: LaneEnd::Dropped,
            ..
        } => "dropped",
        RowKind::Tip { .. } => "thread",
    }
}

fn strokes(arms: Arms, x: usize, y: usize) -> String {
    let (cx, cy) = (x + HALF_W, y + HALF_H);
    [
        (arms.up, format!("M{cx} {cy}V{y}")),
        (arms.down, format!("M{cx} {cy}V{}", y + ROW_H)),
        (arms.left, format!("M{cx} {cy}H{x}")),
        (arms.right, format!("M{cx} {cy}H{}", x + CELL_W)),
    ]
    .into_iter()
    .filter_map(|(present, path)| present.then_some(path))
    .collect()
}

fn shape(glyph: Glyph, x: usize, y: usize, class: &str) -> String {
    let (cx, cy) = (x + HALF_W, y + HALF_H);
    match glyph {
        Glyph::Dot => {
            format!("<circle class=\"dot {class}\" cx=\"{cx}\" cy=\"{cy}\" r=\"{RADIUS}\"/>")
        }
        Glyph::Ring => {
            format!("<circle class=\"{class}\" cx=\"{cx}\" cy=\"{cy}\" r=\"{RADIUS}\"/>")
        }
        Glyph::Cross => format!(
            "<path class=\"{class}\" d=\"M{} {}L{} {}M{} {}L{} {}\"/>",
            cx - RADIUS,
            cy - RADIUS,
            cx + RADIUS,
            cy + RADIUS,
            cx + RADIUS,
            cy - RADIUS,
            cx - RADIUS,
            cy + RADIUS
        ),
        Glyph::Arrow => format!(
            "<path class=\"{class}\" d=\"{}M{} {}L{} {}L{} {}\"/>",
            strokes(
                Arms {
                    left: true,
                    right: true,
                    ..Arms::default()
                },
                x,
                y
            ),
            x + RADIUS + 1,
            cy - RADIUS,
            x + 1,
            cy,
            x + RADIUS + 1,
            cy + RADIUS
        ),
        Glyph::Arms(arms) => format!("<path class=\"{class}\" d=\"{}\"/>", strokes(arms, x, y)),
    }
}

fn row_svg(row: &GraphRow, index: usize) -> String {
    let y = index * ROW_H;
    let body: String = row
        .cells
        .iter()
        .take(row.label_start)
        .enumerate()
        .filter_map(|(column, (ch, tone))| {
            glyph(*ch).map(|glyph| shape(glyph, column * CELL_W, y, tone_class(*tone)))
        })
        .collect();
    format!("<g data-kind=\"{}\">{body}</g>", kind_name(&row.kind))
}

fn columns(rows: &[GraphRow]) -> usize {
    rows.iter()
        .map(|row| row.label_start.min(row.cells.len()))
        .max()
        .unwrap_or(0)
}

pub fn graph_svg(rows: &[GraphRow]) -> Option<String> {
    let columns = columns(rows);
    if rows.is_empty() || columns == 0 {
        return None;
    }
    let (width, height) = (columns * CELL_W, rows.len() * ROW_H);
    let body: String = rows
        .iter()
        .enumerate()
        .map(|(index, row)| row_svg(row, index))
        .collect();
    Some(format!(
        "<r-graph><svg class=\"graph\" role=\"img\" aria-label=\"thread graph, {} rows\" viewBox=\"0 0 {width} {height}\" width=\"{width}\" height=\"{height}\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1\">{body}</svg></r-graph>",
        rows.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::{glyph, graph_svg, Arms, Glyph};
    use crate::render::golden::assert_golden;
    use ralph_core::chat_thread_graph::{graph_cells, GraphRow, Lane, LaneEnd, RowKind, Tone};

    fn lane(name: &str, base: usize, end: LaneEnd) -> Lane {
        Lane {
            name: name.to_string(),
            base,
            end,
        }
    }

    fn rows() -> Vec<GraphRow> {
        let lanes = [
            lane("lint-sweep", 2, LaneEnd::Open),
            lane("park-me", 3, LaneEnd::Parked),
            lane("shipped", 1, LaneEnd::Merged { landed: Some(4) }),
            lane("scrapped", 2, LaneEnd::Dropped),
        ];
        graph_cells(Some(5), &lanes)
    }

    fn row(cells: &[(char, Tone)], label_start: usize) -> GraphRow {
        GraphRow {
            cells: cells.to_vec(),
            label_start,
            kind: RowKind::Trunk { id: 0, head: true },
        }
    }

    #[test]
    fn graph_svg_golden() {
        assert_golden("graph", &graph_svg(&rows()).unwrap());
    }

    #[test]
    fn tones_become_classes_and_no_colour_literal_appears() {
        let svg = graph_svg(&rows()).unwrap();
        for class in ["plain", "green", "yellow", "red"] {
            assert!(svg.contains(&format!("class=\"{class}\"")), "{class}");
        }
        for literal in ["#", "rgb", "hsl", "style=", "fill=\"black\"", "url("] {
            assert!(!svg.contains(literal), "{literal}");
        }
        assert_eq!(svg.matches("stroke=").count(), 1);
        assert!(svg.contains("stroke=\"currentColor\" stroke-width=\"1\""));
    }

    #[test]
    fn row_kinds_are_the_closed_set() {
        let svg = graph_svg(&rows()).unwrap();
        for kind in ["head", "thread", "dropped"] {
            assert!(svg.contains(&format!("data-kind=\"{kind}\"")), "{kind}");
        }
    }

    #[test]
    fn no_rows_or_no_graph_columns_draw_nothing() {
        assert_eq!(graph_svg(&[]), None);
        assert_eq!(graph_svg(&[row(&[('●', Tone::Plain)], 0)]), None);
    }

    #[test]
    fn an_unknown_glyph_draws_nothing_and_keeps_its_column() {
        let svg = graph_svg(&[row(&[('?', Tone::Plain), ('●', Tone::Plain)], 2)]).unwrap();
        assert!(svg.contains("viewBox=\"0 0 24 24\""));
        assert!(svg.contains("cx=\"18\""));
        assert_eq!(svg.matches("<circle").count(), 1);
        assert_eq!(glyph('?'), None);
        assert_eq!(glyph(' '), None);
    }

    #[test]
    fn cells_from_the_label_start_are_not_drawn() {
        let svg = graph_svg(&[row(&[('●', Tone::Plain), ('○', Tone::Red)], 1)]).unwrap();
        assert_eq!(svg.matches("<circle").count(), 1);
        assert!(svg.contains("viewBox=\"0 0 12 24\""));
    }

    #[test]
    fn a_label_start_past_the_cells_is_clamped() {
        let svg = graph_svg(&[row(&[('●', Tone::Plain)], 9)]).unwrap();
        assert!(svg.contains("viewBox=\"0 0 12 24\""));
    }

    #[test]
    fn lane_glyphs_map_to_geometry() {
        assert_eq!(glyph('●'), Some(Glyph::Dot));
        assert_eq!(glyph('○'), Some(Glyph::Ring));
        assert_eq!(glyph('✕'), Some(Glyph::Cross));
        assert_eq!(glyph('◄'), Some(Glyph::Arrow));
        assert_eq!(
            glyph('╯'),
            Some(Glyph::Arms(Arms {
                up: true,
                left: true,
                ..Arms::default()
            }))
        );
        assert_eq!(
            glyph('│'),
            Some(Glyph::Arms(Arms {
                up: true,
                down: true,
                ..Arms::default()
            }))
        );
    }

    #[test]
    fn every_glyph_the_graph_emits_is_drawn() {
        let drawn: Vec<char> = rows()
            .iter()
            .flat_map(|row| row.cells.iter().take(row.label_start))
            .map(|(ch, _)| *ch)
            .filter(|ch| *ch != ' ')
            .collect();
        assert!(!drawn.is_empty());
        for ch in drawn {
            assert!(glyph(ch).is_some(), "{ch}");
        }
    }
}
