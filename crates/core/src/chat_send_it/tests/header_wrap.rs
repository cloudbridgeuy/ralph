use super::render::strip;
use super::*;

const NARROW: usize = 40;

fn wrapped_file() -> RunFile {
    let mut json = plan_json();
    json["steps"] = serde_json::json!([{
        "do": "tidy the imports then rename every helper · and run the tests",
        "criterion": {"kind": "judge", "question": "ok?"}
    }]);
    RunFile {
        plan: parse_run(&json, limits()).unwrap(),
        approved: None,
        stopped: None,
    }
}

#[test]
fn a_wrapped_header_line_with_a_dot_separator_is_bold_and_strips_to_the_plain_text() {
    let file = wrapped_file();
    let styled = render_status_styled(&file, NARROW, TRUST);
    let continuation: Vec<&str> = styled
        .lines()
        .skip_while(|line| !line.contains("1. "))
        .skip(1)
        .take_while(|line| line.starts_with("\x1b[1m"))
        .filter(|line| line.contains(" · "))
        .collect();
    assert!(!continuation.is_empty(), "{styled:?}");
    assert_eq!(strip(&styled), render_status(&file, NARROW, TRUST));
}
