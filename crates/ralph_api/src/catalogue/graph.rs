use super::group;
use crate::render::{empty, error, graph_svg};
use crate::threads::cells::{lane_cells, Lane};

fn dense() -> Vec<Lane> {
    vec![
        Lane::Head,
        Lane::Open,
        Lane::Parked,
        Lane::Merged,
        Lane::Dropped,
    ]
}

fn crowded() -> Vec<Lane> {
    let cycle = [Lane::Open, Lane::Parked, Lane::Merged, Lane::Dropped];
    std::iter::once(Lane::Head)
        .chain(cycle.into_iter().cycle().take(24))
        .collect()
}

fn svg(lanes: &[Lane]) -> String {
    if lanes.is_empty() {
        return empty("No threads");
    }
    lanes
        .iter()
        .enumerate()
        .map(|(index, lane)| graph_svg(&lane_cells(*lane, index + 1 == lanes.len())))
        .collect()
}

pub(super) fn graphs() -> String {
    group(
        "r-graph",
        &[
            ("empty", svg(&[])),
            ("sparse", svg(&[Lane::Head, Lane::Open])),
            ("dense", svg(&dense())),
            ("error", error("could not load the thread graph")),
            ("very-long", svg(&crowded())),
        ],
    )
}
