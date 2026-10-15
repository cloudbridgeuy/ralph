use crate::chat_status::{tokens_label, Memory, Status, TurnCache};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const MODE_LABEL: &str = "IDLE";
const THIN_RULE: &str = "│";
const LEFT_ARROW: &str = "\u{e0b0}";
const LEFT_THIN: &str = "\u{e0b1}";
const RIGHT_ARROW: &str = "\u{e0b2}";
const RIGHT_THIN: &str = "\u{e0b3}";

const MODE_PRIORITY: u8 = 0;
const MAIL_PRIORITY: u8 = 1;
const MEMORY_PRIORITY: u8 = 2;
const CACHE_PRIORITY: u8 = 4;
const COST_PRIORITY: u8 = 3;
const WORKERS_PRIORITY: u8 = 5;
const WATCHES_PRIORITY: u8 = 6;
const STORE_PRIORITY: u8 = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Mode,
    Mail,
    Store,
    Workers,
    Watches,
    Memory,
    Cache,
    Cost,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shade {
    ModeFg,
    ModeBg,
    MailFg,
    MailBg,
    StoreFg,
    WorkersFg,
    WatchesFg,
    MemoryFg,
    CacheFg,
    CacheBg,
    CostFg,
    CostBg,
    BaseBg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Separator {
    Block,
    Powerline,
    Bar,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub fg: Shade,
    pub bg: Shade,
    pub bold: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub variants: Vec<String>,
    pub priority: u8,
    pub side: Side,
    pub kind: Kind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed {
    pub segment: usize,
    pub variant: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Context {
    pub store: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edge {
    Fill,
    Seg(Kind),
}

impl Kind {
    fn bg(self) -> Shade {
        match self {
            Kind::Mode => Shade::ModeBg,
            Kind::Mail => Shade::MailBg,
            Kind::Cache => Shade::CacheBg,
            Kind::Cost => Shade::CostBg,
            Kind::Store | Kind::Workers | Kind::Watches | Kind::Memory => Shade::BaseBg,
        }
    }

    fn fg(self) -> Shade {
        match self {
            Kind::Mode => Shade::ModeFg,
            Kind::Mail => Shade::MailFg,
            Kind::Store => Shade::StoreFg,
            Kind::Workers => Shade::WorkersFg,
            Kind::Watches => Shade::WatchesFg,
            Kind::Memory => Shade::MemoryFg,
            Kind::Cache => Shade::CacheFg,
            Kind::Cost => Shade::CostFg,
        }
    }

    fn bold(self) -> bool {
        matches!(self, Kind::Mode | Kind::Cost)
    }
}

impl Edge {
    fn bg(self) -> Shade {
        match self {
            Edge::Fill => Shade::BaseBg,
            Edge::Seg(kind) => kind.bg(),
        }
    }
}

impl Segment {
    fn new(side: Side, kind: Kind, priority: u8, variants: Vec<String>) -> Self {
        Self {
            variants,
            priority,
            side,
            kind,
        }
    }
}

pub fn store_label(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    parts[parts.len().saturating_sub(2)..].join("/")
}

fn last_component(label: &str) -> String {
    label.rsplit('/').next().unwrap_or(label).to_string()
}

fn dedup(variants: Vec<String>) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for variant in variants {
        if !seen.contains(&variant) {
            seen.push(variant);
        }
    }
    seen
}

fn cache_segments(turn: &TurnCache) -> Vec<Segment> {
    let share = turn.token_share_percent();
    let cache = Segment::new(
        Side::Right,
        Kind::Cache,
        CACHE_PRIORITY,
        vec![
            format!(
                "cache {} read {} write {share}% tok",
                tokens_label(turn.read),
                tokens_label(turn.write)
            ),
            format!("c{share}% tok"),
        ],
    );
    let cost = turn.cost_usd.map(|cost| {
        Segment::new(
            Side::Right,
            Kind::Cost,
            COST_PRIORITY,
            vec![format!("${cost:.2}")],
        )
    });
    std::iter::once(cache).chain(cost).collect()
}

fn workers_variants(status: &Status) -> Vec<String> {
    let count = status.workers.len();
    let names = |with_state: bool| {
        let names: Vec<String> = status
            .workers
            .iter()
            .map(|w| {
                if with_state {
                    format!("{} {}", w.id, w.state)
                } else {
                    w.id.clone()
                }
            })
            .collect();
        format!("workers {}", names.join(", "))
    };
    vec![
        names(true),
        names(false),
        format!("{count} workers"),
        format!("{count}w"),
    ]
}

fn watches_variants(status: &Status) -> Vec<String> {
    let count = status.watches.len();
    let names = |with_left: bool| {
        let names: Vec<String> = status
            .watches
            .iter()
            .map(|watch| match (with_left, &watch.left) {
                (true, Some(left)) => format!("{} {}", watch.id, left),
                _ => watch.id.clone(),
            })
            .collect();
        format!("watch {}", names.join(", "))
    };
    vec![
        names(true),
        names(false),
        format!("{count} watches"),
        format!("{count}wt"),
    ]
}

pub fn segments(status: &Status, context: &Context) -> Vec<Segment> {
    let Memory { msgs, tokens } = status.memory;
    let tok = tokens_label(u64::try_from(tokens).unwrap_or(u64::MAX));
    let unread = status.unread;
    let mode = Segment::new(
        Side::Left,
        Kind::Mode,
        MODE_PRIORITY,
        vec![MODE_LABEL.to_string()],
    );
    let mail = (unread > 0).then(|| {
        Segment::new(
            Side::Left,
            Kind::Mail,
            MAIL_PRIORITY,
            vec![format!("📬 {unread} unread"), format!("📬 {unread}")],
        )
    });
    let store = (!context.store.is_empty()).then(|| {
        Segment::new(
            Side::Left,
            Kind::Store,
            STORE_PRIORITY,
            dedup(vec![context.store.clone(), last_component(&context.store)]),
        )
    });
    let workers = (!status.workers.is_empty()).then(|| {
        Segment::new(
            Side::Right,
            Kind::Workers,
            WORKERS_PRIORITY,
            workers_variants(status),
        )
    });
    let watches = (!status.watches.is_empty()).then(|| {
        Segment::new(
            Side::Right,
            Kind::Watches,
            WATCHES_PRIORITY,
            watches_variants(status),
        )
    });
    let memory = Segment::new(
        Side::Right,
        Kind::Memory,
        MEMORY_PRIORITY,
        vec![
            format!("mem {msgs} msgs ~{tok} tok"),
            format!("{msgs}m ~{tok}"),
        ],
    );
    let cache = status.turn.as_ref().map(cache_segments).unwrap_or_default();
    [Some(mode), mail, store, workers, watches, Some(memory)]
        .into_iter()
        .flatten()
        .chain(cache)
        .collect()
}

fn segment_span(segment: &Segment, variant: usize) -> Span {
    let text = segment.variants.get(variant).map_or("", String::as_str);
    Span {
        text: format!(" {text} "),
        fg: segment.kind.fg(),
        bg: segment.kind.bg(),
        bold: segment.kind.bold(),
    }
}

fn glue(separator: Separator, side: Side, from: Edge, to: Edge) -> Option<Span> {
    let (arrow, thin) = match side {
        Side::Left => (LEFT_ARROW, LEFT_THIN),
        Side::Right => (RIGHT_ARROW, RIGHT_THIN),
    };
    let span = |text: &str, fg: Shade, bg: Shade| {
        Some(Span {
            text: text.to_string(),
            fg,
            bg,
            bold: false,
        })
    };
    match (from, to) {
        (Edge::Seg(a), Edge::Seg(b)) if a.bg() == b.bg() => match separator {
            Separator::Block | Separator::Bar => span(THIN_RULE, a.fg(), a.bg()),
            Separator::Powerline => span(thin, a.fg(), a.bg()),
        },
        (Edge::Seg(_), Edge::Seg(_)) => match separator {
            Separator::Block => None,
            Separator::Bar => span(THIN_RULE, Shade::StoreFg, Shade::BaseBg),
            Separator::Powerline => match side {
                Side::Left => span(arrow, from.bg(), to.bg()),
                Side::Right => span(arrow, to.bg(), from.bg()),
            },
        },
        (Edge::Fill, Edge::Seg(kind)) | (Edge::Seg(kind), Edge::Fill)
            if kind.bg() == Shade::BaseBg =>
        {
            None
        }
        (Edge::Fill, Edge::Seg(_)) | (Edge::Seg(_), Edge::Fill) => match separator {
            Separator::Powerline => match side {
                Side::Left => span(arrow, from.bg(), to.bg()),
                Side::Right => span(arrow, to.bg(), from.bg()),
            },
            Separator::Block | Separator::Bar => None,
        },
        (Edge::Fill, Edge::Fill) => None,
    }
}

fn side_spans(
    segments: &[Segment],
    placed: &[Placed],
    side: Side,
    separator: Separator,
) -> Vec<Span> {
    let items: Vec<(Kind, Span)> = placed
        .iter()
        .filter(|p| segments[p.segment].side == side)
        .map(|p| {
            let segment = &segments[p.segment];
            (segment.kind, segment_span(segment, p.variant))
        })
        .collect();
    let mut spans = Vec::new();
    let mut previous = match side {
        Side::Left => None,
        Side::Right => Some(Edge::Fill),
    };
    for (kind, span) in items {
        let current = Edge::Seg(kind);
        if let Some(glued) = previous.and_then(|from| glue(separator, side, from, current)) {
            spans.push(glued);
        }
        spans.push(span);
        previous = Some(current);
    }
    if side == Side::Left {
        let glued = previous.and_then(|from| glue(separator, side, from, Edge::Fill));
        spans.extend(glued);
    }
    spans
}

fn spans_width(spans: &[Span]) -> usize {
    spans.iter().map(|span| span.text.width()).sum()
}

fn body(segments: &[Segment], placed: &[Placed], separator: Separator) -> (Vec<Span>, Vec<Span>) {
    (
        side_spans(segments, placed, Side::Left, separator),
        side_spans(segments, placed, Side::Right, separator),
    )
}

pub fn layout(segments: &[Segment], width: usize, separator: Separator) -> Vec<Placed> {
    let mut placed: Vec<Placed> = (0..segments.len())
        .map(|segment| Placed {
            segment,
            variant: 0,
        })
        .collect();
    let priority_of = |p: &Placed| segments[p.segment].priority;
    loop {
        let (left, right) = body(segments, &placed, separator);
        if spans_width(&left) + spans_width(&right) <= width {
            return placed;
        }
        let compactable = placed
            .iter()
            .enumerate()
            .filter(|(_, p)| p.variant + 1 < segments[p.segment].variants.len())
            .max_by_key(|(_, p)| priority_of(p))
            .map(|(at, _)| at);
        if let Some(at) = compactable {
            placed[at].variant += 1;
            continue;
        }
        let droppable = placed
            .iter()
            .enumerate()
            .filter(|(_, p)| priority_of(p) > MODE_PRIORITY)
            .max_by_key(|(_, p)| priority_of(p))
            .map(|(at, _)| at);
        match droppable {
            Some(at) => {
                placed.remove(at);
            }
            None => return placed,
        }
    }
}

fn truncate_spans(spans: Vec<Span>, width: usize) -> Vec<Span> {
    let mut remaining = width;
    let mut kept = Vec::new();
    for mut span in spans {
        let mut text = String::new();
        for ch in span.text.chars() {
            let ch_width = ch.width().unwrap_or(0);
            if ch_width > remaining {
                remaining = 0;
                break;
            }
            remaining -= ch_width;
            text.push(ch);
        }
        if text.is_empty() {
            break;
        }
        span.text = text;
        kept.push(span);
        if remaining == 0 {
            break;
        }
    }
    kept
}

pub fn compose(
    segments: &[Segment],
    placed: &[Placed],
    width: usize,
    separator: Separator,
) -> Vec<Span> {
    let (left, right) = body(segments, placed, separator);
    let used = spans_width(&left) + spans_width(&right);
    let fill = Span {
        text: " ".repeat(width.saturating_sub(used)),
        fg: Shade::StoreFg,
        bg: Shade::BaseBg,
        bold: false,
    };
    let all = left
        .into_iter()
        .chain(Some(fill).filter(|fill| !fill.text.is_empty()))
        .chain(right)
        .collect();
    truncate_spans(all, width)
}

pub fn bar(status: &Status, context: &Context, width: usize, separator: Separator) -> Vec<Span> {
    let all = segments(status, context);
    let placed = layout(&all, width, separator);
    compose(&all, &placed, width, separator)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_status::{WatchState, WorkerState};

    const SEPARATORS: [Separator; 3] = [Separator::Block, Separator::Powerline, Separator::Bar];

    fn status() -> Status {
        Status {
            unread: 2,
            workers: vec![
                WorkerState {
                    id: "GUZ-416".into(),
                    state: "running".into(),
                },
                WorkerState {
                    id: "GUZ-417".into(),
                    state: "question".into(),
                },
            ],
            watches: Vec::new(),
            memory: Memory {
                msgs: 132,
                tokens: 4100,
            },
            turn: None,
        }
    }

    fn turn() -> TurnCache {
        TurnCache {
            input: 100,
            read: 12_300,
            write: 400,
            cost_usd: Some(0.0412),
        }
    }

    fn with_turn() -> Status {
        Status {
            turn: Some(turn()),
            ..status()
        }
    }

    fn with_watches() -> Status {
        Status {
            watches: vec![
                WatchState {
                    id: "w1".into(),
                    left: Some("12m".into()),
                },
                WatchState {
                    id: "w2".into(),
                    left: None,
                },
            ],
            ..status()
        }
    }

    fn context() -> Context {
        Context {
            store: ".ralph/chat".into(),
        }
    }

    fn text(spans: &[Span]) -> String {
        spans.iter().map(|span| span.text.as_str()).collect()
    }

    fn line(status: &Status, width: usize) -> String {
        text(&bar(status, &context(), width, Separator::Block))
    }

    fn placed_texts(status: &Status, width: usize) -> Vec<String> {
        let all = segments(status, &context());
        layout(&all, width, Separator::Block)
            .iter()
            .map(|p| all[p.segment].variants[p.variant].clone())
            .collect()
    }

    #[test]
    fn bar_fills_the_full_width_in_every_separator_mode() {
        for separator in SEPARATORS {
            for status in [status(), with_turn(), with_watches()] {
                for width in 12..200 {
                    let spans = bar(&status, &context(), width, separator);
                    assert_eq!(text(&spans).width(), width, "{separator:?} {width}");
                }
            }
        }
    }

    #[test]
    fn bar_never_exceeds_width_even_below_the_mode_block() {
        for separator in SEPARATORS {
            for width in 0..200 {
                let spans = bar(&with_turn(), &context(), width, separator);
                assert!(text(&spans).width() <= width);
            }
        }
        assert!(bar(&status(), &context(), 0, Separator::Block).is_empty());
    }

    #[test]
    fn full_width_shows_every_segment_in_long_form() {
        let all = line(&with_turn(), 200);
        for part in [
            "IDLE",
            "📬 2 unread",
            ".ralph/chat",
            "workers GUZ-416 running, GUZ-417 question",
            "mem 132 msgs ~4.1k tok",
            "cache 12.3k read 400 write 96% tok",
            "$0.04",
        ] {
            assert!(all.contains(part), "{part}: {all}");
        }
    }

    #[test]
    fn layout_degrades_watch_labels_in_order() {
        let status = with_watches();
        let mut seen: Vec<String> = Vec::new();
        for width in (60..300).rev() {
            let texts = placed_texts(&status, width);
            if let Some(watch) = texts
                .iter()
                .find(|t| t.contains("watch") || t.ends_with("wt"))
            {
                if seen.last() != Some(watch) {
                    seen.push(watch.clone());
                }
            }
        }
        assert_eq!(
            seen,
            ["watch w1 12m, w2", "watch w1, w2", "2 watches", "2wt"]
        );
        assert!(placed_texts(&status, 300).contains(&"mem 132 msgs ~4.1k tok".to_string()));
    }

    #[test]
    fn layout_goes_compact_at_forty_columns_on_one_row() {
        let status = with_turn();
        let shown = line(&status, 40);
        assert_eq!(shown.width(), 40);
        assert!(shown.contains("📬 2") && !shown.contains("unread"));
        assert!(shown.contains("$0.04") && !shown.contains("c96%"));
        assert!(shown.contains("132m ~4.1k"));
        assert!(!shown.contains("workers"));
        assert!(!shown.contains(".ralph"));
    }

    #[test]
    fn layout_drops_the_store_then_workers_before_memory() {
        let narrow = placed_texts(&status(), 26);
        assert_eq!(narrow, ["IDLE", "📬 2", "132m ~4.1k"]);
        let wide = placed_texts(&status(), 29);
        assert_eq!(wide, ["IDLE", "📬 2", "2w", "132m ~4.1k"]);
    }

    #[test]
    fn layout_keeps_mail_then_mode_last() {
        let status = status();
        assert_eq!(placed_texts(&status, 12), ["IDLE", "📬 2"]);
        assert_eq!(placed_texts(&status, 11), ["IDLE"]);
        assert_eq!(placed_texts(&status, 6), ["IDLE"]);
        assert_eq!(placed_texts(&status, 0), ["IDLE"]);
    }

    #[test]
    fn zero_unread_has_no_mail_segment_at_any_width() {
        let status = Status {
            unread: 0,
            ..with_turn()
        };
        for width in 0..200 {
            assert!(!line(&status, width).contains("📬"));
        }
        assert!(line(&status, 200).starts_with(" IDLE "));
        assert_eq!(
            line(&status, 24).split_whitespace().collect::<Vec<_>>(),
            ["IDLE", "132m", "~4.1k"]
        );
    }

    #[test]
    fn one_unread_keeps_mail_at_every_form() {
        let status = Status {
            unread: 1,
            ..status()
        };
        assert!(line(&status, 200).contains("📬 1 unread"));
        assert!(line(&status, 30).contains("📬 1"));
        assert!(line(&status, 12).contains("📬 1"));
    }

    #[test]
    fn workers_and_watches_are_absent_when_empty() {
        let none = Status {
            workers: Vec::new(),
            ..status()
        };
        let all = line(&none, 200);
        assert!(!all.contains("worker") && !all.contains("watch"));
    }

    #[test]
    fn cache_omits_cost_when_missing_and_share_ignores_cost() {
        for cost_usd in [Some(0.0), Some(10.0), None] {
            let status = Status {
                turn: Some(TurnCache {
                    input: 100,
                    read: 300,
                    write: 200,
                    cost_usd,
                }),
                ..status()
            };
            assert!(line(&status, 200).contains("cache 300 read 200 write 50% tok"));
            assert!(line(&status, 50).contains("c50% tok") || cost_usd.is_some());
            assert_eq!(line(&status, 200).contains('$'), cost_usd.is_some());
        }
    }

    #[test]
    fn segment_kinds_run_in_display_order_on_the_right() {
        let all = segments(&with_turn(), &context());
        let right: Vec<Kind> = all
            .iter()
            .filter(|s| s.side == Side::Right)
            .map(|s| s.kind)
            .collect();
        assert_eq!(
            right,
            [Kind::Workers, Kind::Memory, Kind::Cache, Kind::Cost]
        );
        assert_eq!(all[0].kind, Kind::Mode);
        assert_eq!(all[0].priority, 0);
    }

    #[test]
    fn powerline_glyphs_appear_only_in_powerline_mode() {
        let glyphs = [LEFT_ARROW, RIGHT_ARROW, LEFT_THIN, RIGHT_THIN];
        let has = |separator| {
            let shown = text(&bar(&with_turn(), &context(), 200, separator));
            glyphs.iter().any(|glyph| shown.contains(glyph))
        };
        assert!(has(Separator::Powerline));
        assert!(!has(Separator::Block));
        assert!(!has(Separator::Bar));
    }

    #[test]
    fn mode_block_is_bold_ink_on_accent() {
        let spans = bar(&status(), &context(), 80, Separator::Block);
        assert_eq!(
            spans[0],
            Span {
                text: " IDLE ".into(),
                fg: Shade::ModeFg,
                bg: Shade::ModeBg,
                bold: true
            }
        );
        assert_eq!(spans[1].bg, Shade::MailBg);
        let filler = spans.iter().find(|s| s.text.trim().is_empty()).unwrap();
        assert_eq!(filler.bg, Shade::BaseBg);
    }

    #[test]
    fn powerline_arrow_blends_neighbouring_backgrounds() {
        let spans = bar(&status(), &context(), 80, Separator::Powerline);
        assert_eq!(spans[1].text, LEFT_ARROW);
        assert_eq!((spans[1].fg, spans[1].bg), (Shade::ModeBg, Shade::MailBg));
    }

    #[test]
    fn bar_separator_puts_a_rule_between_segments() {
        let shown = text(&bar(&status(), &context(), 200, Separator::Bar));
        assert!(shown.starts_with(" IDLE │ 📬 2 unread │ "));
    }

    #[test]
    fn store_label_keeps_the_last_two_components() {
        assert_eq!(store_label("/home/me/.ralph/chat"), ".ralph/chat");
        assert_eq!(store_label("chat"), "chat");
        assert_eq!(store_label("/chat/"), "chat");
        assert_eq!(store_label(""), "");
    }

    #[test]
    fn empty_store_label_has_no_store_segment() {
        let all = segments(&status(), &Context::default());
        assert!(all.iter().all(|s| s.priority != STORE_PRIORITY));
    }

    #[test]
    fn single_component_store_has_one_variant() {
        let all = segments(
            &status(),
            &Context {
                store: "chat".into(),
            },
        );
        let store = all.iter().find(|s| s.priority == STORE_PRIORITY).unwrap();
        assert_eq!(store.variants, ["chat"]);
    }
}
