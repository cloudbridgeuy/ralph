use crate::stream_processor::StreamProcessor;

const STEP_RESULT_USE: &str = r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"t1","name":"mcp__ralph-chat__step_result","input":{"met":true,"evidence":"ok"}}]}}"#;
const BASH_USE: &str = r#"{"type":"assistant","message":{"id":"m2","content":[{"type":"tool_use","id":"t2","name":"Bash","input":{"command":"ls"}}]}}"#;

fn result_line(id: &str, content: &str, is_error: bool) -> String {
    format!(
        r#"{{"type":"user","message":{{"id":"u","content":[{{"type":"tool_result","tool_use_id":"{id}","content":"{content}","is_error":{is_error}}}]}}}}"#
    )
}

fn shown(lines: &[&str]) -> String {
    let mut processor = StreamProcessor::with_options(false, true);
    lines
        .iter()
        .filter_map(|line| processor.process_line(line))
        .collect()
}

#[test]
fn step_result_invocation_is_hidden() {
    assert!(!shown(&[STEP_RESULT_USE]).contains("step_result"));
}

#[test]
fn step_result_success_result_is_hidden() {
    let ok = result_line("t1", "recorded", false);
    assert!(!shown(&[STEP_RESULT_USE, &ok]).contains("recorded"));
}

#[test]
fn step_result_error_result_is_shown() {
    let failed = result_line("t1", "judge turn only", true);
    assert!(shown(&[STEP_RESULT_USE, &failed]).contains("judge turn only"));
}

#[test]
fn other_tool_invocation_is_still_shown() {
    assert!(shown(&[BASH_USE]).contains("ls"));
}
