use super::*;

pub(super) const MODE_PRIORITY: u8 = 0;
const QUEUE_PRIORITY: u8 = 1;
const MAIL_PRIORITY: u8 = 2;
const MEMORY_PRIORITY: u8 = 3;
const CACHE_PRIORITY: u8 = 5;
const COST_PRIORITY: u8 = 4;
const WORKERS_PRIORITY: u8 = 6;
const WATCHES_PRIORITY: u8 = 7;
const STORE_PRIORITY: u8 = 8;

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

fn cache_segments(turn: &TurnCache, current: Mode) -> Vec<Segment> {
    let share = turn.token_share_percent();
    let cache = Segment::new(
        Side::Right,
        Kind::Cache(current),
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
            Kind::Cost(current),
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
    let current = mode(status);
    let mode = Segment::new(
        Side::Left,
        Kind::Mode(current),
        MODE_PRIORITY,
        vec![current.label().to_string()],
    );
    let queue = status.queue.map(|cell| {
        Segment::new(
            Side::Left,
            Kind::Queue(current),
            QUEUE_PRIORITY,
            queue_variants(cell),
        )
    });
    let mail = (unread > 0).then(|| {
        Segment::new(
            Side::Left,
            Kind::Mail(current),
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
    let cache = status
        .turn
        .as_ref()
        .map(|turn| cache_segments(turn, current))
        .unwrap_or_default();
    [
        Some(mode),
        queue,
        mail,
        store,
        workers,
        watches,
        Some(memory),
    ]
    .into_iter()
    .flatten()
    .chain(cache)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::*;
    use super::*;
    use crate::status_queue::QueueCell;

    #[test]
    fn full_width_shows_every_segment_in_long_form() {
        let all = line(&with_turn(), 200);
        for part in [
            "INBOX",
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
    fn queue_cell_follows_the_mode_and_outlasts_mail() {
        let status = Status {
            queue: Some(QueueCell {
                running: true,
                count: 2,
                held: false,
            }),
            ..status()
        };
        assert!(line(&status, 200).starts_with(" INBOX  turn running · QUEUED 2  📬 2 unread"));
        assert_eq!(placed_texts(&status, 20), ["INBOX", "▶ Q2", "📬 2"]);
        assert_eq!(placed_texts(&status, 14), ["INBOX", "▶ Q2"]);
    }

    #[test]
    fn no_queue_cell_outside_the_queue_editor() {
        assert!(!line(&status(), 200).contains("turn"));
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
        assert!(line(&status, 200).starts_with(" WORK "));
        assert_eq!(
            line(&status, 24).split_whitespace().collect::<Vec<_>>(),
            ["WORK", "132m", "~4.1k"]
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
        assert!(line(&status, 13).contains("📬 1"));
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
            [
                Kind::Workers,
                Kind::Memory,
                Kind::Cache(Mode::Inbox),
                Kind::Cost(Mode::Inbox)
            ]
        );
        assert_eq!(all[0].kind, Kind::Mode(Mode::Inbox));
        assert_eq!(all[0].priority, 0);
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
