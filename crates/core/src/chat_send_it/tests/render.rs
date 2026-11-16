use super::*;

const WIDE: usize = 1000;
const NARROW: usize = 40;
const PLAN_HASH: &str = "b40c6519589a";

pub(super) fn strip(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            chars.by_ref().find(|next| *next == 'm');
        } else {
            out.push(c);
        }
    }
    out
}

fn step_plan(task: &str, criterion: serde_json::Value) -> RunPlan {
    let mut json = plan_json();
    json["steps"] = serde_json::json!([{"do": task, "criterion": criterion}]);
    parse_run(&json, limits()).unwrap()
}

fn header(plan: &RunPlan) -> String {
    render_run(plan)
        .lines()
        .find(|line| line.starts_with("  1. "))
        .unwrap()
        .to_string()
}

fn file() -> RunFile {
    RunFile {
        plan: plan(),
        approved: None,
        stopped: None,
    }
}

#[test]
fn the_restatement_gives_each_step_a_header_a_brief_and_an_exit_line() {
    assert_eq!(
        render_run(&plan()),
        "Goal: ship GUZ-1\n\
         Steps:\n  \
           1. worker · collect evidence\n       \
                collect evidence\n     \
              exit: worker E1 reports done\n  \
           2. check · wait for CI\n       \
                wait for CI\n     \
              exit: watch `gh pr checks --watch` exits 0\n     \
              ralph runs this check alone: put any work in an earlier step\n  \
           3. check · run the checks\n       \
                run the checks\n     \
              exit: `cargo xtask lint` exits 0\n     \
              ralph runs this check alone: put any work in an earlier step\n  \
           4. judge · address review\n       \
                address review\n     \
              exit: judged: are all findings resolved?\n\
         Allowed: spawn codex workers, merge, paid runs up to $2.50\n\
         Not allowed: reply to workers, close workers, start watches, push, open PRs, write to Linear\n\
         Warning: step 1 waits for worker E1, but the plan does not allow `reply`. A question from that worker stops the run.\n\
         Caps: 3 turns, $4.00, 2 judge rounds\n\
         You pre-approve:\n  \
           - raise the turn cap to 30"
    );
}

#[test]
fn a_restatement_without_actions_says_so() {
    let mut plan = plan();
    plan.allowed.clear();
    let text = render_run(&plan);
    assert!(text.contains("\nAllowed: none\n"), "{text}");
}

#[test]
fn the_header_kind_follows_the_exit_criterion() {
    let worker = serde_json::json!({"kind": "worker_done", "worker": "E1"});
    let watch = serde_json::json!({"kind": "watch_passes", "command": "gh pr checks"});
    let check = serde_json::json!({"kind": "check_passes", "command": "make"});
    let judge = serde_json::json!({"kind": "judge", "question": "ok?"});
    for (criterion, kind) in [
        (worker, "worker"),
        (watch, "check"),
        (check, "check"),
        (judge, "judge"),
    ] {
        let text = header(&step_plan("do it", criterion));
        assert_eq!(text, format!("  1. {kind} · do it"));
    }
}

#[test]
fn a_worker_step_shows_worker_whatever_its_task_says() {
    let worker = serde_json::json!({"kind": "worker_done", "worker": "E1"});
    for task in ["merge the branch", "Reply to the worker", "wait for E1"] {
        let text = header(&step_plan(task, worker.clone()));
        assert_eq!(text, format!("  1. worker · {task}"));
    }
}

#[test]
fn no_spawn_header_shows_for_a_plan_that_does_not_allow_spawn() {
    let worker = serde_json::json!({"kind": "worker_done", "worker": "E1"});
    let mut plan = step_plan("wait for E1", worker);
    plan.allowed
        .retain(|action| !matches!(action, Action::Spawn { .. }));
    let text = render_run(&plan);
    assert!(!text.contains("spawn ·"), "{text}");
    assert!(text.contains("1. worker · wait for E1"), "{text}");
}

#[test]
fn the_title_is_the_first_line_and_the_brief_keeps_every_line_indented() {
    let judge = serde_json::json!({"kind": "judge", "question": "ok?"});
    let task = format!("fix the bug\nthen {}\nend", "word ".repeat(40));
    let text = render_run(&step_plan(&task, judge));
    assert_eq!(header_of(&text), "  1. judge · fix the bug");
    let brief: Vec<&str> = text
        .lines()
        .skip_while(|line| !line.starts_with("  1. "))
        .skip(1)
        .take_while(|line| !line.trim_start().starts_with("exit:"))
        .collect();
    assert!(brief.len() >= 4, "{text}");
    assert!(
        brief.iter().all(|line| line.starts_with("       ")),
        "{text}"
    );
    assert!(brief.iter().all(|line| line.len() <= 75), "{text}");
    assert_eq!(brief[0].trim(), "fix the bug");
    assert_eq!(brief.last().unwrap().trim(), "end");
    let words = brief
        .iter()
        .flat_map(|line| line.split_whitespace())
        .count();
    assert_eq!(words, 3 + 1 + 40 + 1, "{text}");
}

#[test]
fn a_task_that_starts_with_a_blank_line_is_titled_by_its_first_text_line() {
    let judge = serde_json::json!({"kind": "judge", "question": "ok?"});
    let mut plan = step_plan("x", judge);
    plan.steps[0].task = "\n  \n  fix the bug  \nmore".to_string();
    let text = render_run(&plan);
    assert_eq!(header_of(&text), "  1. judge · fix the bug");
}

fn header_of(text: &str) -> &str {
    text.lines().find(|line| line.starts_with("  1. ")).unwrap()
}

#[test]
fn the_exit_sits_on_its_own_line_after_the_brief() {
    let text = render_run(&plan());
    let lines: Vec<&str> = text.lines().collect();
    let exit = lines
        .iter()
        .position(|line| line.starts_with("     exit: "));
    assert_eq!(
        lines[exit.unwrap() - 1],
        "       collect evidence",
        "{text}"
    );
}

#[test]
fn the_styled_status_has_ansi_and_the_plain_status_has_none() {
    let styled = render_status_styled(&file(), WIDE);
    assert!(styled.contains('\x1b'), "{styled:?}");
    assert!(!render_status(&file(), WIDE).contains('\x1b'));
    assert!(!render_run(&plan()).contains('\x1b'));
}

#[test]
fn stripping_the_styled_status_gives_the_plain_status() {
    for width in [WIDE, NARROW] {
        let styled = render_status_styled(&file(), width);
        assert_eq!(strip(&styled), render_status(&file(), width));
    }
}

#[test]
fn the_exit_command_is_in_code_style_and_the_label_is_dim() {
    let styled = render_status_styled(&file(), WIDE);
    assert!(
        styled.contains("\x1b[2mexit:\x1b[0m \x1b[36m`cargo xtask lint`\x1b[0m exits 0"),
        "{styled:?}"
    );
}

#[test]
fn a_command_wrapped_over_two_lines_is_styled_on_both() {
    let long = step_plan(
        "x",
        serde_json::json!({"kind": "check_passes", "command": "cargo test --workspace --all-features now"}),
    );
    let file = RunFile {
        plan: long,
        approved: None,
        stopped: None,
    };
    let styled = render_status_styled(&file, NARROW);
    let lines: Vec<&str> = styled
        .lines()
        .skip_while(|line| !line.contains("exit:"))
        .take(2)
        .collect();
    assert!(lines.len() >= 2, "{styled:?}");
    for line in lines {
        assert_eq!(line.matches("\x1b[36m").count(), 1, "{line:?}");
        assert!(line.contains("\x1b[0m"), "{line:?}");
    }
}

#[test]
fn the_plan_hash_does_not_depend_on_the_display() {
    assert_eq!(plan_hash(&plan()), PLAN_HASH);
}

#[test]
fn the_brief_the_model_sees_has_no_escape_character() {
    let started = start(&run_file_approved(), &[], &[]).unwrap();
    let Next::Send(brief) = started.next else {
        panic!("{:?}", started.next)
    };
    assert!(brief.contains("1. worker · collect evidence"), "{brief}");
    assert!(!brief.contains('\x1b'), "{brief:?}");
}

fn run_file_approved() -> RunFile {
    RunFile {
        approved: Some(plan_hash(&plan())),
        ..file()
    }
}
