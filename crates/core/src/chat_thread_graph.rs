use crate::ansi::{GREEN, RED, RESET, YELLOW};
use crate::chat_thread::ThreadMeta;
use crate::chat_thread_log::Tombstone;
use std::collections::BTreeSet;
use unicode_width::UnicodeWidthChar;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaneEnd {
    Open,
    Merged { landed: Option<usize> },
    Parked,
    Dropped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lane {
    pub name: String,
    pub base: usize,
    pub end: LaneEnd,
}

#[derive(Debug, Clone, Copy)]
pub struct GraphOpts {
    pub width: usize,
    pub color: bool,
}

pub fn graph_opts(color: bool, columns: usize) -> GraphOpts {
    GraphOpts {
        width: if color && columns > 0 {
            columns
        } else {
            usize::MAX
        },
        color,
    }
}

fn end_of(meta: &ThreadMeta) -> LaneEnd {
    match (&meta.merged, &meta.parked) {
        (Some(merged), _) => LaneEnd::Merged {
            landed: merged.landed,
        },
        (None, Some(_)) => LaneEnd::Parked,
        (None, None) => LaneEnd::Open,
    }
}

pub fn lanes_from(threads: &[(String, ThreadMeta)], tombstones: &[Tombstone]) -> Vec<Lane> {
    let live = threads.iter().map(|(name, meta)| Lane {
        name: name.clone(),
        base: meta.base,
        end: end_of(meta),
    });
    let dropped = tombstones.iter().map(|tombstone| Lane {
        name: tombstone.name.clone(),
        base: tombstone.base,
        end: LaneEnd::Dropped,
    });
    live.chain(dropped).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Green,
    Yellow,
    Red,
}

pub type Cell = (char, Tone);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Tip(usize),
    Trunk(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Link {
    Fork,
    Merge,
}

#[derive(Debug, Clone, Copy)]
struct Span {
    top: usize,
    base: usize,
}

impl LaneEnd {
    fn tone(&self) -> Tone {
        match self {
            LaneEnd::Open => Tone::Plain,
            LaneEnd::Merged { .. } => Tone::Green,
            LaneEnd::Parked => Tone::Yellow,
            LaneEnd::Dropped => Tone::Red,
        }
    }

    fn tip(&self) -> char {
        match self {
            LaneEnd::Dropped => '✕',
            _ => '○',
        }
    }

    fn status(&self, compact: bool) -> String {
        match (self, compact) {
            (LaneEnd::Open, _) => "open".to_string(),
            (LaneEnd::Parked, _) => "parked".to_string(),
            (LaneEnd::Dropped, _) => "dropped".to_string(),
            (LaneEnd::Merged { landed: Some(id) }, false) => format!("merged -> #{id}"),
            (LaneEnd::Merged { landed: Some(id) }, true) => format!("-> #{id}"),
            (LaneEnd::Merged { landed: None }, false) => "merged, landing unknown".to_string(),
            (LaneEnd::Merged { landed: None }, true) => "merged?".to_string(),
        }
    }
}

impl Tone {
    fn code(self) -> Option<&'static str> {
        match self {
            Tone::Plain => None,
            Tone::Green => Some(GREEN),
            Tone::Yellow => Some(YELLOW),
            Tone::Red => Some(RED),
        }
    }
}

fn landing_row(lane: &Lane) -> Option<usize> {
    match lane.end {
        LaneEnd::Merged { landed: Some(id) } => Some(id.max(lane.base)),
        _ => None,
    }
}

fn ordered(lanes: &[Lane]) -> Vec<&Lane> {
    let mut sorted: Vec<&Lane> = lanes.iter().collect();
    sorted.sort_by(|a, b| (a.base, &a.name).cmp(&(b.base, &b.name)));
    sorted
}

fn rows_of(head: Option<usize>, lanes: &[&Lane]) -> Vec<Row> {
    let mut ids = BTreeSet::from([0]);
    ids.extend(head);
    for lane in lanes {
        ids.insert(lane.base);
        ids.extend(landing_row(lane));
    }
    let tips = lanes
        .iter()
        .enumerate()
        .filter(|(_, lane)| landing_row(lane).is_none())
        .map(|(i, _)| Row::Tip(i))
        .rev();
    tips.chain(ids.into_iter().rev().map(Row::Trunk)).collect()
}

fn span_of(i: usize, lane: &Lane, rows: &[Row]) -> Span {
    let at = |wanted: Row| rows.iter().position(|row| *row == wanted).unwrap_or(0);
    let top = match landing_row(lane) {
        Some(id) => at(Row::Trunk(id)),
        None => at(Row::Tip(i)),
    };
    Span {
        top,
        base: at(Row::Trunk(lane.base)),
    }
}

fn cell_width(c: char) -> usize {
    UnicodeWidthChar::width(c).unwrap_or(0)
}

fn text_width(text: &str) -> usize {
    text.chars().map(cell_width).sum()
}

fn truncated(text: &str, room: usize) -> String {
    if text_width(text) <= room {
        return text.to_string();
    }
    let keep = room.saturating_sub(1);
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        if used + cell_width(c) > keep {
            break;
        }
        used += cell_width(c);
        out.push(c);
    }
    if room > 0 {
        out.push('…');
    }
    out
}

fn cells(text: &str, tone: Tone) -> Vec<Cell> {
    text.chars().map(|c| (c, tone)).collect()
}

fn lane_label(lane: &Lane, budget: usize) -> Vec<Cell> {
    let full = lane.end.status(false);
    let (name, gap, status) = if text_width(&lane.name) + 2 + text_width(&full) <= budget {
        (lane.name.clone(), "  ", full)
    } else {
        let short = lane.end.status(true);
        let room = budget.saturating_sub(text_width(&short) + 1);
        (truncated(&lane.name, room), " ", short)
    };
    let mut label = cells(&name, Tone::Plain);
    label.extend(cells(gap, Tone::Plain));
    label.extend(cells(&status, lane.end.tone()));
    label
}

fn label_of(head: Option<usize>, row: Row, lanes: &[&Lane], budget: usize) -> Vec<Cell> {
    match row {
        Row::Tip(i) => lane_label(lanes[i], budget),
        Row::Trunk(id) => {
            let mut label = cells(&format!("#{id}"), Tone::Plain);
            if head == Some(id) {
                label.extend(cells(" main", Tone::Plain));
            }
            for lane in lanes.iter().filter(|lane| landing_row(lane) == Some(id)) {
                let room = budget.saturating_sub(label.len() + 2);
                label.extend(cells("  ", Tone::Plain));
                label.extend(lane_label(lane, room));
            }
            label
        }
    }
}

fn lane_glyph(
    r: usize,
    row: Row,
    i: usize,
    lane: &Lane,
    span: Span,
) -> Option<(char, Option<Link>)> {
    let merging = landing_row(lane).is_some();
    match row {
        Row::Tip(t) if t == i => Some((lane.end.tip(), None)),
        Row::Trunk(_) if merging && r == span.top && r == span.base => {
            Some(('╯', Some(Link::Merge)))
        }
        Row::Trunk(_) if merging && r == span.top => Some(('╮', Some(Link::Merge))),
        Row::Trunk(_) if r == span.base => Some(('╯', Some(Link::Fork))),
        _ if span.top < r && r < span.base => Some(('│', None)),
        _ => None,
    }
}

fn row_cells(r: usize, row: Row, lanes: &[&Lane], spans: &[Span]) -> Vec<Cell> {
    let mut graph = vec![(' ', Tone::Plain); 2 * (lanes.len() + 1)];
    if matches!(row, Row::Trunk(_)) {
        graph[0] = ('●', Tone::Plain);
    }
    let mut links: Vec<(usize, Link)> = Vec::new();
    for (i, lane) in lanes.iter().enumerate() {
        if let Some((glyph, link)) = lane_glyph(r, row, i, lane, spans[i]) {
            graph[2 * (i + 1)] = (glyph, lane.end.tone());
            links.extend(link.map(|link| (i, link)));
        }
    }
    let Some(&(far, _)) = links.last() else {
        return graph;
    };
    let tone = lanes[far].end.tone();
    let merging = links.iter().any(|(_, link)| *link == Link::Merge);
    for (pos, cell) in graph.iter_mut().enumerate().take(2 * (far + 1)).skip(1) {
        let link = links.iter().find(|(i, _)| 2 * (i + 1) == pos);
        *cell = match (pos, link) {
            (_, Some((_, Link::Fork))) => ('┴', cell.1),
            (_, Some((_, Link::Merge))) => ('┬', cell.1),
            (_, None) if pos % 2 == 0 && cell.0 == '│' => ('┼', cell.1),
            (1, None) if merging => ('◄', tone),
            _ => ('─', tone),
        };
    }
    graph
}

fn clipped(mut line: Vec<Cell>, width: usize) -> Vec<Cell> {
    while line.last().is_some_and(|(c, _)| *c == ' ') {
        line.pop();
    }
    let mut used = 0;
    line.retain(|(c, _)| {
        used += cell_width(*c);
        used <= width
    });
    line
}

fn painted(line: &[Cell], color: bool) -> String {
    let mut out = String::new();
    let mut tone = Tone::Plain;
    for &(c, next) in line {
        if color && next != tone {
            if tone != Tone::Plain {
                out.push_str(RESET);
            }
            out.extend(next.code());
            tone = next;
        }
        out.push(c);
    }
    if color && tone != Tone::Plain {
        out.push_str(RESET);
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowKind {
    Trunk { id: usize, head: bool },
    Tip { name: String, end: LaneEnd },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRow {
    pub cells: Vec<Cell>,
    pub label_start: usize,
    pub kind: RowKind,
}

fn kind_of(head: Option<usize>, row: Row, lanes: &[&Lane]) -> RowKind {
    match row {
        Row::Tip(i) => RowKind::Tip {
            name: lanes[i].name.clone(),
            end: lanes[i].end.clone(),
        },
        Row::Trunk(id) => RowKind::Trunk {
            id,
            head: head == Some(id),
        },
    }
}

fn build_rows(head: Option<usize>, lanes: &[Lane], width: usize) -> Vec<GraphRow> {
    let lanes = ordered(lanes);
    let rows = rows_of(head, &lanes);
    let spans: Vec<Span> = lanes
        .iter()
        .enumerate()
        .map(|(i, lane)| span_of(i, lane, &rows))
        .collect();
    let graph_width = 2 * (lanes.len() + 1) + 1;
    let budget = width.saturating_sub(graph_width);
    rows.iter()
        .enumerate()
        .map(|(r, &row)| {
            let mut line = row_cells(r, row, &lanes, &spans);
            line.push((' ', Tone::Plain));
            let label_start = line.len();
            line.extend(label_of(head, row, &lanes, budget));
            GraphRow {
                cells: clipped(line, width),
                label_start,
                kind: kind_of(head, row, &lanes),
            }
        })
        .collect()
}

pub fn graph_cells(head: Option<usize>, lanes: &[Lane]) -> Vec<GraphRow> {
    if head.is_none() && lanes.is_empty() {
        return vec![GraphRow {
            cells: vec![('●', Tone::Plain)],
            label_start: 1,
            kind: RowKind::Trunk { id: 0, head: false },
        }];
    }
    build_rows(head, lanes, usize::MAX)
}

pub fn graph_lines(head: Option<usize>, lanes: &[Lane], opts: &GraphOpts) -> Vec<String> {
    if head.is_none() && lanes.is_empty() {
        return vec!["●".to_string()];
    }
    build_rows(head, lanes, opts.width)
        .iter()
        .map(|row| painted(&row.cells, opts.color))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ansi::{GREEN, RED, YELLOW};
    use crate::chat_thread::{Merged, Parked};
    use unicode_width::UnicodeWidthStr;

    fn lane(name: &str, base: usize, end: LaneEnd) -> Lane {
        Lane {
            name: name.to_string(),
            base,
            end,
        }
    }

    fn plain(width: usize) -> GraphOpts {
        GraphOpts {
            width,
            color: false,
        }
    }

    fn sample() -> Vec<Lane> {
        vec![
            lane("a", 3, LaneEnd::Merged { landed: Some(5) }),
            lane("b", 3, LaneEnd::Parked),
            lane("c", 4, LaneEnd::Dropped),
            lane("d", 4, LaneEnd::Open),
        ]
    }

    #[test]
    fn graph_cells_text_equals_plain_graph_lines() {
        let lanes = [
            lane("alpha", 3, LaneEnd::Merged { landed: Some(5) }),
            lane("beta", 3, LaneEnd::Parked),
            lane("gamma", 4, LaneEnd::Dropped),
            lane("delta", 4, LaneEnd::Open),
        ];
        let rows = graph_cells(Some(6), &lanes);
        let text: Vec<String> = rows
            .iter()
            .map(|row| row.cells.iter().map(|(c, _)| *c).collect())
            .collect();
        assert_eq!(text, graph_lines(Some(6), &lanes, &plain(usize::MAX)));
        assert_eq!(rows.len(), 8);
        assert_eq!(
            rows[0].kind,
            RowKind::Tip {
                name: "gamma".to_string(),
                end: LaneEnd::Dropped
            }
        );
        assert_eq!(rows[3].kind, RowKind::Trunk { id: 6, head: true });
        assert_eq!(rows[0].cells[8], ('✕', Tone::Red));
    }

    #[test]
    fn a_mixed_history_draws_newest_first() {
        let lines = graph_lines(Some(6), &sample(), &plain(100));
        let expected = [
            "        ○  d  open",
            "      ✕ │  c  dropped",
            "    ○ │ │  b  parked",
            "●   │ │ │  #6 main",
            "●◄╮ │ │ │  #5  a  merged -> #5",
            "●─┼─┼─┴─╯  #4",
            "●─┴─╯      #3",
            "●          #0",
        ];
        assert_eq!(lines, expected);
    }

    #[test]
    fn a_merged_lane_names_its_landing() {
        let lanes = [lane("a", 3, LaneEnd::Merged { landed: Some(5) })];
        let text = graph_lines(Some(6), &lanes, &plain(100)).join("\n");
        assert!(text.contains("◄"), "{text}");
        assert!(text.contains("a  merged -> #5"), "{text}");
    }

    #[test]
    fn a_merged_lane_without_a_landing_says_so() {
        let lanes = [lane("a", 3, LaneEnd::Merged { landed: None })];
        let text = graph_lines(Some(6), &lanes, &plain(100)).join("\n");
        assert!(text.contains("a  merged, landing unknown"), "{text}");
        assert!(!text.contains("◄"), "{text}");
    }

    #[test]
    fn a_parked_lane_ends_in_a_tip() {
        let lanes = [lane("b", 3, LaneEnd::Parked)];
        let text = graph_lines(Some(6), &lanes, &plain(100)).join("\n");
        assert!(text.contains("○  b  parked"), "{text}");
    }

    #[test]
    fn a_dropped_lane_hangs_off_its_tombstone_base() {
        let lanes = [lane("c", 4, LaneEnd::Dropped)];
        let lines = graph_lines(Some(6), &lanes, &plain(100));
        assert!(lines[0].contains("✕  c  dropped"), "{lines:?}");
        let fork = lines.iter().find(|line| line.contains("#4")).unwrap();
        assert!(fork.starts_with("●─╯"), "{fork}");
    }

    #[test]
    fn an_open_lane_says_open() {
        let lanes = [lane("d", 4, LaneEnd::Open)];
        let text = graph_lines(Some(6), &lanes, &plain(100)).join("\n");
        assert!(text.contains("○  d  open"), "{text}");
    }

    #[test]
    fn lane_order_does_not_depend_on_input_order() {
        let mut reversed = sample();
        reversed.reverse();
        assert_eq!(
            graph_lines(Some(6), &sample(), &plain(100)),
            graph_lines(Some(6), &reversed, &plain(100))
        );
    }

    #[test]
    fn no_color_prints_no_escape() {
        let text = graph_lines(Some(6), &sample(), &plain(100)).join("\n");
        assert!(!text.contains('\u{1b}'));
    }

    #[test]
    fn color_paints_merged_green_parked_yellow_and_dropped_red() {
        let opts = GraphOpts {
            width: 100,
            color: true,
        };
        let lines = graph_lines(Some(6), &sample(), &opts);
        let line = |needle: &str| lines.iter().find(|l| l.contains(needle)).unwrap().clone();
        assert!(line("merged").contains(&format!("{GREEN}merged")));
        assert!(line("parked").contains(&format!("{YELLOW}parked")));
        assert!(line("dropped").contains(&format!("{RED}dropped")));
        assert!(!line("open").contains(GREEN));
    }

    #[test]
    fn narrow_output_never_exceeds_the_width() {
        let lanes = vec![
            lane(
                "a-very-long-thread-name",
                3,
                LaneEnd::Merged { landed: Some(500) },
            ),
            lane(
                "another-long-thread-name",
                3,
                LaneEnd::Merged { landed: None },
            ),
            lane("parked-thread-with-a-long-name", 4, LaneEnd::Parked),
            lane("dropped-thread-with-a-long-name", 4, LaneEnd::Dropped),
            lane("open-thread-with-a-long-name", 5, LaneEnd::Open),
        ];
        for width in [30, 20, 12, 5, 1] {
            for color in [false, true] {
                for line in graph_lines(Some(600), &lanes, &GraphOpts { width, color }) {
                    let shown = strip(&line);
                    assert!(shown.width() <= width, "{width}: {shown:?}");
                }
            }
        }
    }

    #[test]
    fn narrow_output_shortens_labels_with_an_ellipsis() {
        let lanes = [lane(
            "a-very-long-thread-name",
            2,
            LaneEnd::Merged { landed: Some(4) },
        )];
        let text = graph_lines(Some(5), &lanes, &plain(30)).join("\n");
        assert!(text.contains('…'), "{text}");
        assert!(text.contains("-> #4"), "{text}");
    }

    #[test]
    fn an_empty_history_is_just_the_trunk() {
        assert_eq!(graph_lines(None, &[], &plain(80)), vec!["●"]);
    }

    #[test]
    fn main_labels_the_last_row_and_a_fork_at_the_end_shows_its_base_without_it() {
        let lanes = [lane("a", 5, LaneEnd::Open)];
        let lines = graph_lines(Some(4), &lanes, &plain(100));
        assert_eq!(lines.len(), 4, "{lines:?}");
        assert!(lines[1].ends_with("#5"), "{lines:?}");
        assert!(lines[2].ends_with("#4 main"), "{lines:?}");
    }

    #[test]
    fn a_lane_on_an_empty_main_hangs_off_row_zero_without_main() {
        let lanes = [lane("a", 0, LaneEnd::Open)];
        let text = graph_lines(None, &lanes, &plain(100)).join("\n");
        assert!(text.contains("#0"), "{text}");
        assert!(!text.contains("main"), "{text}");
    }

    fn meta(base: usize) -> ThreadMeta {
        ThreadMeta {
            base,
            created: "c".to_string(),
            merged: None,
            parked: None,
            auto: None,
            model: None,
            closed: false,
        }
    }

    fn tombstone(name: &str, base: usize) -> Tombstone {
        Tombstone {
            name: name.to_string(),
            base,
            created: "c".to_string(),
            dropped_at: "d".to_string(),
        }
    }

    #[test]
    fn lanes_come_from_each_thread_state_and_each_tombstone() {
        let merged = ThreadMeta {
            merged: Some(Merged {
                at: "t".to_string(),
                digest: "d".to_string(),
                landed: Some(9),
            }),
            ..meta(2)
        };
        let unknown = ThreadMeta {
            merged: Some(Merged {
                at: "t".to_string(),
                digest: "d".to_string(),
                landed: None,
            }),
            ..meta(3)
        };
        let parked = ThreadMeta {
            parked: Some(Parked {
                at: "t".to_string(),
            }),
            ..meta(4)
        };
        let threads = [
            ("m".to_string(), merged),
            ("u".to_string(), unknown),
            ("p".to_string(), parked),
            ("o".to_string(), meta(5)),
        ];
        let lanes = lanes_from(&threads, &[tombstone("x", 6)]);
        assert_eq!(
            lanes,
            vec![
                lane("m", 2, LaneEnd::Merged { landed: Some(9) }),
                lane("u", 3, LaneEnd::Merged { landed: None }),
                lane("p", 4, LaneEnd::Parked),
                lane("o", 5, LaneEnd::Open),
                lane("x", 6, LaneEnd::Dropped),
            ]
        );
    }

    #[test]
    fn no_threads_and_no_tombstones_give_no_lanes() {
        assert_eq!(lanes_from(&[], &[]), Vec::new());
    }

    #[test]
    fn colour_fits_the_terminal_and_plain_output_keeps_every_column() {
        let tty = graph_opts(true, 80);
        assert_eq!((tty.width, tty.color), (80, true));
        let piped = graph_opts(false, 80);
        assert_eq!((piped.width, piped.color), (usize::MAX, false));
    }

    #[test]
    fn a_terminal_that_reports_no_width_is_not_clipped_to_nothing() {
        let opts = graph_opts(true, 0);
        assert_eq!((opts.width, opts.color), (usize::MAX, true));
    }

    fn strip(line: &str) -> String {
        let mut shown = String::new();
        let mut skipping = false;
        for c in line.chars() {
            match (skipping, c) {
                (_, '\u{1b}') => skipping = true,
                (true, 'm') => skipping = false,
                (false, c) => shown.push(c),
                _ => {}
            }
        }
        shown
    }
}
