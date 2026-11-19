use super::*;

const PREFERENCE: &str = "Prefer check_passes or watch_passes: ralph runs them for $0.";
const FALLBACK: &str = "Use judge only as a fallback: it costs one model turn per round.";

#[test]
fn the_propose_prompt_prefers_check_and_watch_over_judge() {
    let prompt = propose_prompt("ship GUZ-1");
    assert!(prompt.contains(PREFERENCE), "{prompt}");
    assert!(prompt.contains(FALLBACK), "{prompt}");
}

#[test]
fn the_propose_run_tool_prefers_check_and_watch_over_judge() {
    let tool = propose_run_tool();
    let criterion = tool["inputSchema"]["properties"]["steps"]["items"]["properties"]["criterion"]
        ["description"]
        .as_str()
        .unwrap();
    assert!(criterion.contains(PREFERENCE), "{criterion}");
    assert!(criterion.contains(FALLBACK), "{criterion}");
    let step = tool["inputSchema"]["properties"]["steps"]["items"]["properties"]["do"]
        ["description"]
        .as_str()
        .unwrap();
    assert!(step.contains("check_passes"), "{step}");
}
