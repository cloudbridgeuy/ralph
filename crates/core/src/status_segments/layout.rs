use super::build::MODE_PRIORITY;
use super::*;

const THIN_RULE: &str = "│";
const LEFT_ARROW: &str = "\u{e0b0}";
const LEFT_THIN: &str = "\u{e0b1}";
const RIGHT_ARROW: &str = "\u{e0b2}";
const RIGHT_THIN: &str = "\u{e0b3}";

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
            Separator::Block => None,
            Separator::Bar => span(THIN_RULE, a.fg(), a.bg()),
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
    use super::super::fixtures::*;
    use super::*;

    const SEPARATORS: [Separator; 3] = [Separator::Block, Separator::Powerline, Separator::Bar];

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
        let narrow = placed_texts(&status(), 27);
        assert_eq!(narrow, ["INBOX", "📬 2", "132m ~4.1k"]);
        let wide = placed_texts(&status(), 30);
        assert_eq!(wide, ["INBOX", "📬 2", "2w", "132m ~4.1k"]);
    }

    #[test]
    fn layout_keeps_mail_then_mode_last() {
        let status = status();
        assert_eq!(placed_texts(&status, 13), ["INBOX", "📬 2"]);
        assert_eq!(placed_texts(&status, 12), ["INBOX"]);
        assert_eq!(placed_texts(&status, 7), ["INBOX"]);
        assert_eq!(placed_texts(&status, 0), ["INBOX"]);
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
                text: " INBOX ".into(),
                fg: Shade::ModeInboxFg,
                bg: Shade::ModeInboxBg,
                bold: true
            }
        );
        assert_eq!(spans[1].bg, Shade::ModeInboxBg);
        let filler = spans.iter().find(|s| s.text.trim().is_empty()).unwrap();
        assert_eq!(filler.bg, Shade::BaseBg);
    }

    #[test]
    fn powerline_arrow_blends_neighbouring_backgrounds() {
        let spans = bar(&status(), &context(), 80, Separator::Powerline);
        let at = spans.iter().position(|s| s.text == LEFT_ARROW).unwrap();
        assert_eq!(
            (spans[at].fg, spans[at].bg),
            (Shade::ModeInboxBg, Shade::BaseBg)
        );
    }

    #[test]
    fn bar_separator_puts_a_rule_between_segments() {
        let shown = text(&bar(&status(), &context(), 200, Separator::Bar));
        assert!(shown.starts_with(" INBOX │ 📬 2 unread │ "));
    }

    #[test]
    fn mail_cache_and_cost_take_the_mode_colours() {
        let expected = [
            (Mode::Idle, Shade::ModeIdleBg, Shade::ModeIdleFg),
            (Mode::Nap, Shade::ModeNapBg, Shade::ModeNapFg),
            (Mode::Inbox, Shade::ModeInboxBg, Shade::ModeInboxFg),
            (Mode::Work, Shade::ModeWorkBg, Shade::ModeWorkFg),
        ];
        for (mode, bg, fg) in expected {
            for kind in [Kind::Mail(mode), Kind::Cache(mode), Kind::Cost(mode)] {
                assert_eq!((kind.bg(), kind.fg()), (bg, fg));
            }
        }
    }

    #[test]
    fn middle_blocks_keep_the_base_background_and_their_own_text() {
        assert_eq!(
            [Kind::Store, Kind::Workers, Kind::Watches, Kind::Memory].map(|k| (k.bg(), k.fg())),
            [
                (Shade::BaseBg, Shade::StoreFg),
                (Shade::BaseBg, Shade::WorkersFg),
                (Shade::BaseBg, Shade::WatchesFg),
                (Shade::BaseBg, Shade::MemoryFg),
            ]
        );
    }

    #[test]
    fn same_colour_neighbours_have_no_separator_in_block_mode() {
        let spans = bar(&status(), &context(), 80, Separator::Block);
        assert_eq!(spans[0].text, " INBOX ");
        assert_eq!(spans[1].text, " 📬 2 unread ");
        assert_eq!(spans[0].bg, spans[1].bg);
    }

    #[test]
    fn powerline_same_colour_neighbours_use_the_thin_glyph() {
        let spans = bar(&status(), &context(), 80, Separator::Powerline);
        assert_eq!(spans[1].text, LEFT_THIN);
    }

    #[test]
    fn only_mode_and_cost_segments_are_bold() {
        assert!(Kind::Mode(Mode::Idle).bold());
        assert!(Kind::Cost(Mode::Idle).bold());
        assert!(!Kind::Memory.bold());
        assert!(!Kind::Store.bold());
    }

    #[test]
    fn powerline_glue_to_the_fill_depends_on_the_segment_background() {
        let coloured = Edge::Seg(Kind::Mode(Mode::Work));
        let plain = Edge::Seg(Kind::Memory);
        assert!(glue(Separator::Powerline, Side::Left, coloured, Edge::Fill).is_some());
        assert!(glue(Separator::Powerline, Side::Left, plain, Edge::Fill).is_none());
        assert!(glue(Separator::Powerline, Side::Left, Edge::Fill, plain).is_none());
    }

    #[test]
    fn right_side_has_no_trailing_glue() {
        let segments = vec![Segment {
            variants: vec!["x".into()],
            priority: 1,
            side: Side::Right,
            kind: Kind::Cost(Mode::Work),
        }];
        let placed = [Placed {
            segment: 0,
            variant: 0,
        }];
        let spans = side_spans(&segments, &placed, Side::Right, Separator::Powerline);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[1].text, " x ");
    }
}
