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

fn dense_state() -> String {
    [
        svg(&dense()),
        svg(&[Lane::Head, Lane::Parked]),
        svg(&[Lane::Head, Lane::Merged]),
    ]
    .concat()
}

pub(super) fn graphs() -> String {
    group(
        "r-graph",
        &[
            ("empty", svg(&[])),
            ("sparse", svg(&[Lane::Head, Lane::Open])),
            ("dense", dense_state()),
            ("error", error("could not load the thread graph")),
            ("very-long", svg(&crowded())),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::graphs;

    const CURVE: &str = "M4 -1V8Q4 12 8 12";

    fn svgs() -> Vec<String> {
        graphs()
            .split("</svg>")
            .filter_map(|part| part.find("<svg").map(|start| part[start..].to_string()))
            .collect()
    }

    fn has_curve_with(class: &str) -> bool {
        svgs()
            .iter()
            .any(|svg| svg.contains(CURVE) && svg.contains(class))
    }

    #[test]
    fn dense_shows_the_last_row_curve_for_parked_and_merged() {
        assert!(has_curve_with("ln warn h"));
        assert!(has_curve_with("ln ok h"));
    }
}
