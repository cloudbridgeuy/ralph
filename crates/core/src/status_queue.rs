/// Turn state shown in the status bar while the queue editor is open.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QueueCell {
    pub running: bool,
    pub count: usize,
    pub held: bool,
}

pub fn queue_variants(cell: QueueCell) -> Vec<String> {
    let (state, mark) = if cell.running {
        ("running", "▶")
    } else {
        ("done", "✓")
    };
    let (label, short) = if cell.held {
        ("HELD", "H")
    } else {
        ("QUEUED", "Q")
    };
    match cell.count {
        0 => vec![format!("turn {state}"), state.to_string(), mark.to_string()],
        count => vec![
            format!("turn {state} · {label} {count}"),
            format!("{state} · {short}{count}"),
            format!("{mark} {short}{count}"),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_turn_with_entries_shows_the_count() {
        assert_eq!(
            queue_variants(QueueCell {
                running: true,
                count: 2,
                held: false,
            }),
            ["turn running · QUEUED 2", "running · Q2", "▶ Q2"]
        );
    }

    #[test]
    fn finished_turn_shows_done() {
        assert_eq!(
            queue_variants(QueueCell {
                running: false,
                count: 3,
                held: false,
            }),
            ["turn done · QUEUED 3", "done · Q3", "✓ Q3"]
        );
    }

    #[test]
    fn empty_queue_shows_only_the_turn_state() {
        assert_eq!(
            queue_variants(QueueCell {
                running: true,
                count: 0,
                held: false,
            }),
            ["turn running", "running", "▶"]
        );
        assert_eq!(
            queue_variants(QueueCell {
                running: false,
                count: 0,
                held: false,
            }),
            ["turn done", "done", "✓"]
        );
    }

    #[test]
    fn held_queue_says_held_in_every_form() {
        assert_eq!(
            queue_variants(QueueCell {
                running: true,
                count: 2,
                held: true,
            }),
            ["turn running · HELD 2", "running · H2", "▶ H2"]
        );
        assert_eq!(
            queue_variants(QueueCell {
                running: false,
                count: 2,
                held: true,
            }),
            ["turn done · HELD 2", "done · H2", "✓ H2"]
        );
    }
}
