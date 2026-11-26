use super::group;
use crate::render::{empty, error, graph_svg};
use ralph_core::chat_thread_graph::{graph_cells, Lane, LaneEnd};

fn lane(name: &str, base: usize, end: LaneEnd) -> Lane {
    Lane {
        name: name.to_string(),
        base,
        end,
    }
}

fn dense() -> Vec<Lane> {
    vec![
        lane("lint-sweep", 2, LaneEnd::Open),
        lane("park-me", 3, LaneEnd::Parked),
        lane("shipped", 1, LaneEnd::Merged { landed: Some(4) }),
        lane("scrapped", 2, LaneEnd::Dropped),
    ]
}

fn crowded() -> Vec<Lane> {
    (0..24)
        .map(|index| {
            let end = match index % 4 {
                0 => LaneEnd::Open,
                1 => LaneEnd::Parked,
                2 => LaneEnd::Dropped,
                _ => LaneEnd::Merged {
                    landed: Some(index + 2),
                },
            };
            lane(&format!("side-quest-{index}"), index / 2, end)
        })
        .collect()
}

fn svg(head: Option<usize>, lanes: &[Lane]) -> String {
    graph_svg(&graph_cells(head, lanes)).unwrap_or_else(|| empty("No threads"))
}

pub(super) fn graphs() -> String {
    group(
        "r-graph",
        &[
            ("empty", svg(None, &[])),
            (
                "sparse",
                svg(Some(2), &[lane("lint-sweep", 1, LaneEnd::Open)]),
            ),
            ("dense", svg(Some(5), &dense())),
            ("error", error("could not load the thread graph")),
            ("very-long", svg(Some(40), &crowded())),
        ],
    )
}
