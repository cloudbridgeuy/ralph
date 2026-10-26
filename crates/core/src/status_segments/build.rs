use super::*;
use crate::chat_thread::{cost_variants, thread_variants};

pub(super) const MODE_PRIORITY: u8 = 0;
const QUEUE_PRIORITY: u8 = 1;
const MAIL_PRIORITY: u8 = 2;
const THREAD_PRIORITY: u8 = 3;
const COST_PRIORITY: u8 = 4;
const WORKERS_PRIORITY: u8 = 5;
const WATCHES_PRIORITY: u8 = 6;
const STORE_PRIORITY: u8 = 7;

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
    let hold = status.hold.as_ref().map(|filter| {
        let label = filter.label();
        Segment::new(
            Side::Left,
            Kind::Hold(current),
            QUEUE_PRIORITY,
            vec![format!("from {label}"), label.to_string()],
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
    let thread = status.thread.as_ref().map(|cell| {
        Segment::new(
            Side::Right,
            Kind::Thread(current),
            THREAD_PRIORITY,
            thread_variants(cell),
        )
    });
    let cost = status
        .thread
        .as_ref()
        .and_then(|cell| cell.cost.as_ref())
        .map(|cost| {
            Segment::new(
                Side::Right,
                Kind::Cost(current),
                COST_PRIORITY,
                cost_variants(cost),
            )
        });
    [
        Some(mode),
        queue,
        hold,
        mail,
        store,
        workers,
        watches,
        thread,
        cost,
    ]
    .into_iter()
    .flatten()
    .collect()
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::*;
    use super::*;
    use crate::chat_hold::HoldFilter;
    use crate::chat_thread::ThreadCell;
    use crate::status_queue::QueueCell;

    #[test]
    fn full_width_shows_every_segment_in_long_form() {
        let all = line(&with_thread(), 200);
        for part in [
            "INBOX",
            "📬 2 unread",
            ".ralph/chat",
            "workers GUZ-416 running, GUZ-417 question",
            "T12/30 · ctx 62% · cache 41m · shift ~9 msg",
            "$4.12 (+0.31)",
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
    fn hold_cell_names_the_source_in_hold_mode() {
        let status = Status {
            hold: Some(HoldFilter::Source("GUZ-9".into())),
            ..status()
        };
        assert!(line(&status, 200).starts_with(" HOLD  from GUZ-9  📬 2 unread"));
        assert_eq!(placed_texts(&status, 20), ["HOLD", "GUZ-9", "📬 2"]);
        let any = Status {
            hold: Some(HoldFilter::Any),
            ..status
        };
        assert!(line(&any, 200).starts_with(" HOLD  from anyone "));
    }

    #[test]
    fn no_hold_cell_without_a_hold() {
        assert!(!line(&status(), 200).contains("from"));
    }

    #[test]
    fn no_queue_cell_outside_the_queue_editor() {
        assert!(!line(&status(), 200).contains("turn"));
    }

    #[test]
    fn zero_unread_has_no_mail_segment_at_any_width() {
        let status = Status {
            unread: 0,
            ..with_thread()
        };
        for width in 0..200 {
            assert!(!line(&status, width).contains("📬"));
        }
        assert!(line(&status, 200).starts_with(" WORK "));
        assert_eq!(
            line(&status, 24).split_whitespace().collect::<Vec<_>>(),
            ["WORK", "2w", "41m", "$4.12"]
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
    fn cost_is_absent_without_a_session_cost() {
        let status = Status {
            thread: Some(ThreadCell {
                cost: None,
                ..thread()
            }),
            ..status()
        };
        assert!(line(&status, 200).contains("cache 41m"));
        assert!(!line(&status, 200).contains('$'));
        assert!(!line(&Status::default(), 200).contains("cache"));
    }

    #[test]
    fn segment_kinds_run_in_display_order_on_the_right() {
        let all = segments(&with_thread(), &context());
        let right: Vec<Kind> = all
            .iter()
            .filter(|s| s.side == Side::Right)
            .map(|s| s.kind)
            .collect();
        assert_eq!(
            right,
            [
                Kind::Workers,
                Kind::Thread(Mode::Inbox),
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
