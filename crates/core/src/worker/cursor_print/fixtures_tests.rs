use serde_json::Value;

const SUCCESS: &str = include_str!("fixtures/success.jsonl");
const TOOL: &str = include_str!("fixtures/tool.jsonl");
const INTERRUPTED: &str = include_str!("fixtures/interrupted.jsonl");
const CRASH: &str = include_str!("fixtures/crash.jsonl");

const USAGE_KEYS: [&str; 4] = [
    "inputTokens",
    "outputTokens",
    "cacheReadTokens",
    "cacheWriteTokens",
];

fn lines(text: &str) -> Vec<Value> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn kind(line: &Value) -> &str {
    line["type"].as_str().unwrap()
}

fn has_result(lines: &[Value]) -> bool {
    lines.iter().any(|line| kind(line) == "result")
}

fn assert_ends_with_usage(lines: &[Value]) {
    let last = lines.last().unwrap();
    assert_eq!(kind(last), "result");
    for key in USAGE_KEYS {
        assert!(last["usage"][key].is_u64(), "missing usage key {key}");
    }
}

fn assert_one_session(lines: &[Value]) {
    let first = lines[0]["session_id"].as_str().unwrap();
    assert!(!first.is_empty());
    for line in lines {
        assert_eq!(line["session_id"].as_str(), Some(first));
    }
}

#[test]
fn every_fixture_line_is_json_with_a_type() {
    for text in [SUCCESS, TOOL, INTERRUPTED, CRASH] {
        let parsed = lines(text);
        assert!(!parsed.is_empty());
        for line in &parsed {
            assert!(line["type"].is_string());
        }
        assert_eq!(kind(&parsed[0]), "system");
    }
}

#[test]
fn success_and_tool_end_with_a_result_carrying_usage() {
    assert_ends_with_usage(&lines(SUCCESS));
    assert_ends_with_usage(&lines(TOOL));
}

#[test]
fn tool_fixture_has_started_then_completed() {
    let subtypes: Vec<String> = lines(TOOL)
        .iter()
        .filter(|line| kind(line) == "tool_call")
        .map(|line| line["subtype"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(subtypes, ["started", "completed"]);
}

#[test]
fn interrupted_and_crash_have_no_result() {
    assert!(!has_result(&lines(INTERRUPTED)));
    assert!(!has_result(&lines(CRASH)));
}

#[test]
fn every_line_shares_one_session_id() {
    for text in [SUCCESS, TOOL, INTERRUPTED, CRASH] {
        assert_one_session(&lines(text));
    }
}
