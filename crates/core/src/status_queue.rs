/// Turn state shown in the status bar while the queue editor is open.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QueueCell {
    pub running: bool,
    pub count: usize,
}

pub fn queue_variants(cell: QueueCell) -> Vec<String> {
    let (state, mark) = if cell.running {
        ("running", "▶")
    } else {
        ("done", "✓")
    };
    match cell.count {
        0 => vec![format!("turn {state}"), state.to_string(), mark.to_string()],
        count => vec![
            format!("turn {state} · QUEUED {count}"),
            format!("{state} · Q{count}"),
            format!("{mark} Q{count}"),
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
                count: 2
            }),
            ["turn running · QUEUED 2", "running · Q2", "▶ Q2"]
        );
    }

    #[test]
    fn finished_turn_shows_done() {
        assert_eq!(
            queue_variants(QueueCell {
                running: false,
                count: 3
            }),
            ["turn done · QUEUED 3", "done · Q3", "✓ Q3"]
        );
    }

    #[test]
    fn empty_queue_shows_only_the_turn_state() {
        assert_eq!(
            queue_variants(QueueCell {
                running: true,
                count: 0
            }),
            ["turn running", "running", "▶"]
        );
        assert_eq!(
            queue_variants(QueueCell {
                running: false,
                count: 0
            }),
            ["turn done", "done", "✓"]
        );
    }
}
