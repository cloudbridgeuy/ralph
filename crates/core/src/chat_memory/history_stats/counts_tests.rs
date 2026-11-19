use super::counts::{count_calls, percent};
use super::parse::Tool;
use super::test_rows::{call, history};
use serde_json::json;

#[test]
fn counts_calls_per_tool_and_source() {
    let parsed = history(&[
        call(json!({"tool": "search", "source": "mcp"})),
        call(json!({"tool": "search", "source": "slash"})),
        call(json!({"tool": "zoom", "source": "mcp", "outcome": "empty"})),
        call(json!({"tool": "read", "source": "cli", "outcome": "error"})),
        call(json!({"tool": "mystery", "source": "pigeon", "outcome": "huh"})),
    ]);
    let counts = count_calls(&parsed.calls);
    let find = |tool| counts.tools.iter().find(|row| row.tool == tool).unwrap();
    let search = find(Tool::Search);
    assert_eq!(
        (search.total, search.mcp, search.slash, search.cli),
        (2, 1, 1, 0)
    );
    assert_eq!(search.ok, 2);
    let zoom = find(Tool::Zoom);
    assert_eq!((zoom.total, zoom.mcp, zoom.empty), (1, 1, 1));
    let read = find(Tool::Read);
    assert_eq!((read.cli, read.error), (1, 1));
    assert_eq!(find(Tool::Recall).total, 0);
    let other = find(Tool::Unknown);
    assert_eq!(
        (other.total, other.other_source, other.other_outcome),
        (1, 1, 1)
    );
}

#[test]
fn rates_empty_and_error_per_tool() {
    let parsed = history(&[
        call(json!({"outcome": "empty"})),
        call(json!({"outcome": "empty"})),
        call(json!({"outcome": "error"})),
        call(json!({"outcome": "ok"})),
    ]);
    let counts = count_calls(&parsed.calls);
    let search = counts
        .tools
        .iter()
        .find(|row| row.tool == Tool::Search)
        .unwrap();
    assert_eq!((search.empty_pct(), search.error_pct()), (50, 25));
    let recall = counts
        .tools
        .iter()
        .find(|row| row.tool == Tool::Recall)
        .unwrap();
    assert_eq!((recall.empty_pct(), recall.error_pct()), (0, 0));
}

#[test]
fn percent_rounds_and_guards_zero() {
    assert_eq!(percent(1, 3), 33);
    assert_eq!(percent(2, 3), 67);
    assert_eq!(percent(5, 0), 0);
}
