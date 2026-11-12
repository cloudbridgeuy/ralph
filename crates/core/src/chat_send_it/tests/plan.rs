use super::*;

#[test]
fn usage_names_the_instructions_form_and_the_default_caps() {
    let text = usage(&Limits {
        max_turns: 15,
        max_usd: 5.0,
    });
    assert!(text.starts_with("Usage: /send-it <instructions>"), "{text}");
    assert!(text.contains("exit criterion"), "{text}");
    assert!(text.contains("Default caps: 15 turns, $5.00."), "{text}");
    assert!(
        text.ends_with("/send-it templates, /send-it <ticket> <name>."),
        "{text}"
    );
}

#[test]
fn send_it_keywords_count_only_as_the_whole_argument() {
    assert_eq!(parse_send_it_args(""), SendItCommand::Usage);
    assert_eq!(parse_send_it_args("  "), SendItCommand::Usage);
    assert_eq!(parse_send_it_args(" edit "), SendItCommand::Edit);
    assert_eq!(parse_send_it_args("go"), SendItCommand::Go);
    assert_eq!(parse_send_it_args("status"), SendItCommand::Status);
    assert_eq!(parse_send_it_args("off"), SendItCommand::Off);
    assert_eq!(
        parse_send_it_args(" go ship GUZ-1 "),
        SendItCommand::Propose("go ship GUZ-1".to_string())
    );
}

fn run_file(approved: Option<String>) -> RunFile {
    RunFile {
        plan: plan(),
        approved,
        stopped: None,
    }
}

#[test]
fn a_run_file_round_trips_through_toml_with_its_approval() {
    for approved in [None, Some(plan_hash(&plan()))] {
        let file = run_file(approved);
        let text = write_run(&file).unwrap();
        assert_eq!(read_run(&text, limits()), Ok(file), "{text}");
    }
    assert!(!write_run(&run_file(None)).unwrap().contains("approved"));
}

#[test]
fn a_run_file_round_trips_its_stopped_step_outside_the_plan_hash() {
    let hash = plan_hash(&plan());
    let file = RunFile {
        stopped: Some(Stopped {
            step: 2,
            hash: hash.clone(),
        }),
        ..run_file(None)
    };
    let text = write_run(&file).unwrap();
    assert!(text.contains("[stopped]"), "{text}");
    let read = read_run(&text, limits()).unwrap();
    assert_eq!(read, file, "{text}");
    assert_eq!(plan_hash(&read.plan), hash);
    assert!(!write_run(&run_file(None)).unwrap().contains("stopped"));
}

const HAND_WRITTEN: &str = r#"
goal = "ship GUZ-1"

[[steps]]
do = "run the checks"
criterion = { kind = "check_passes", command = "cargo xtask lint" }
"#;

#[test]
fn a_hand_written_run_file_takes_the_default_caps() {
    let file = read_run(HAND_WRITTEN, limits()).unwrap();
    assert_eq!(file.plan.limits, limits());
    assert_eq!(file.plan.judge_rounds, DEFAULT_JUDGE_ROUNDS);
    assert_eq!(file.approved, None);
}

#[test]
fn a_bad_run_file_says_what_is_wrong() {
    assert!(read_run("goal = ", limits())
        .unwrap_err()
        .starts_with("run.toml is malformed"));
    let no_criterion = "goal = \"g\"\n[[steps]]\ndo = \"x\"\n";
    assert_eq!(
        read_run(no_criterion, limits()),
        Err("Step 1 has no exit criterion. Give each step a check ralph can measure.".to_string())
    );
}

#[test]
fn the_plan_hash_ignores_layout_and_follows_content() {
    let hand = read_run(HAND_WRITTEN, limits()).unwrap();
    let hash = plan_hash(&hand.plan);
    assert_eq!(hash.len(), 12);
    assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    let rewritten = read_run(&write_run(&hand).unwrap(), limits()).unwrap();
    assert_eq!(plan_hash(&rewritten.plan), hash);
    let mut changed = hand.plan.clone();
    changed.steps[0].task = "run the tests".to_string();
    assert_ne!(plan_hash(&changed), hash);
}

#[test]
fn go_approves_only_the_shown_plan_typed_at_the_prompt() {
    let file = run_file(None);
    let hash = plan_hash(&file.plan);
    assert!(!file.is_approved());
    assert!(approve(&file, Some(&hash), false)
        .unwrap_err()
        .contains("type it at the prompt"));
    for shown in [None, Some("0123456789ab")] {
        assert!(approve(&file, shown, true)
            .unwrap_err()
            .contains("/send-it status"));
    }
    let approved = approve(&file, Some(&hash), true).unwrap();
    assert_eq!(approved.approved, Some(hash));
    assert!(approved.is_approved());
}

#[test]
fn an_edited_plan_loses_its_approval() {
    let mut file = approve(&run_file(None), Some(&plan_hash(&plan())), true).unwrap();
    assert!(file.is_approved());
    file.plan.steps.pop();
    assert!(!file.is_approved());
}

#[test]
fn status_shows_the_plan_its_hash_and_whether_it_is_approved() {
    let file = run_file(None);
    let hash = plan_hash(&file.plan);
    let status = render_status(&file, WIDE);
    assert!(status.starts_with(&render_run(&file.plan)), "{status}");
    assert!(status.ends_with(&format!(
        "Plan {hash}: not approved. Type /send-it go to approve this plan."
    )));
    let approved = approve(&file, Some(&hash), true).unwrap();
    assert!(render_status(&approved, WIDE).ends_with(&format!("Plan {hash}: approved.")));
    let stopped = RunFile {
        stopped: Some(Stopped {
            step: 2,
            hash: hash.clone(),
        }),
        ..file
    };
    assert!(render_status(&stopped, WIDE).ends_with(&format!(
        "Plan {hash}: stopped at step 2/4. Type /send-it go to resume from step 2."
    )));
}

#[test]
fn only_an_approved_run_blocks_a_new_proposal() {
    let draft = run_file(None);
    assert_eq!(propose_refusal(None), None);
    assert_eq!(propose_refusal(Some(&draft)), None);
    let hash = plan_hash(&draft.plan);
    let approved = approve(&draft, Some(&hash), true).unwrap();
    assert_eq!(
        propose_refusal(Some(&approved)),
        Some(format!(
            "Plan {hash} is approved and active. Stop it with /send-it off first."
        ))
    );
}

#[test]
fn the_propose_prompt_asks_for_one_propose_run_call_and_no_work() {
    let prompt = propose_prompt("ship GUZ-1 and merge");
    assert!(prompt.ends_with("\n\nship GUZ-1 and merge"), "{prompt}");
    assert!(prompt.contains("Call `propose_run` once"), "{prompt}");
    assert!(prompt.contains("Do not start the work"), "{prompt}");
}

#[test]
fn the_propose_run_tool_requires_a_criterion_on_every_step() {
    let tool = propose_run_tool();
    assert_eq!(tool["name"], "propose_run");
    let schema = &tool["inputSchema"];
    assert_eq!(schema["required"], serde_json::json!(["goal", "steps"]));
    assert_eq!(
        schema["properties"]["steps"]["items"]["required"],
        serde_json::json!(["do", "criterion"])
    );
}

const WIDE: usize = 1000;
const NARROW: usize = 40;

fn long_file() -> RunFile {
    let plan = parse_run(
        &serde_json::json!({
            "goal": "ship GUZ-1 with every review finding resolved and the release notes updated",
            "steps": [
                {"do": "collect evidence about the failing wrap in the send-it status screen", "criterion": {"kind": "judge", "question": "does every line of the plan fit inside a forty column terminal?"}}
            ],
            "allowed": [],
            "grants": []
        }),
        limits(),
    )
    .unwrap();
    RunFile {
        plan,
        approved: None,
        stopped: None,
    }
}

fn continuations<'a>(status: &'a str, first: &str) -> Vec<&'a str> {
    status
        .lines()
        .skip_while(|line| !line.starts_with(first))
        .skip(1)
        .take_while(|line| line.starts_with("  ") && !line.trim_start().starts_with("exit:"))
        .collect()
}

#[test]
fn status_wraps_long_steps_at_word_boundaries() {
    let file = long_file();
    let status = render_status(&file, NARROW);
    assert!(
        status.lines().all(|line| line.chars().count() <= NARROW),
        "{status}"
    );
    let words = |text: &str| {
        text.split_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    let wide = render_status(&file, WIDE);
    assert_eq!(words(&status), words(&wide), "{status}");
}

#[test]
fn a_wrapped_step_continues_under_its_text() {
    let status = render_status(&long_file(), NARROW);
    let rest = continuations(&status, "  1. ");
    assert!(!rest.is_empty(), "{status}");
    for line in rest {
        assert_eq!(line.len() - line.trim_start().len(), 5, "{status}");
    }
}

#[test]
fn a_wrapped_exit_continues_under_its_criterion() {
    let status = render_status(&long_file(), NARROW);
    let rest = continuations(&status, "     exit: ");
    assert!(!rest.is_empty(), "{status}");
    for line in rest {
        assert_eq!(line.len() - line.trim_start().len(), 11, "{status}");
    }
}

#[test]
fn a_wide_screen_shows_the_plan_unchanged() {
    let file = long_file();
    let hash = plan_hash(&file.plan);
    assert_eq!(
        render_status(&file, WIDE),
        format!(
            "{}\nPlan {hash}: not approved. Type /send-it go to approve this plan.",
            render_run(&file.plan)
        )
    );
}

const CHECK_RULE: &str = "run by ralph alone, with no model turn";

#[test]
fn the_propose_prompt_and_tool_say_a_check_step_is_run_by_ralph_alone() {
    let prompt = propose_prompt("ship GUZ-1");
    assert!(prompt.contains(CHECK_RULE), "{prompt}");
    assert!(prompt.contains("earlier step"), "{prompt}");
    let tool = propose_run_tool();
    let criterion = tool["inputSchema"]["properties"]["steps"]["items"]["properties"]["criterion"]
        ["description"]
        .as_str()
        .unwrap();
    assert!(criterion.contains(CHECK_RULE), "{criterion}");
    assert!(criterion.contains("earlier step"), "{criterion}");
}

#[test]
fn only_a_check_or_watch_step_carries_the_run_by_ralph_note() {
    let text = render_run(&plan());
    assert_eq!(text.matches("ralph runs this check alone").count(), 2);
    let first_note = text.find("ralph runs this check alone").unwrap();
    assert!(first_note > text.find("2. wait for CI").unwrap());
    assert!(text.find("1. collect evidence").unwrap() < first_note);
    assert!(!text[..text.find("2. wait for CI").unwrap()].contains("ralph runs this check alone"));
}
