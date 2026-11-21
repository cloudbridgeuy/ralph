use super::render::strip;
use super::*;

const WIDE: usize = 1000;
const NARROW: usize = 40;
const EVERYTHING: &str = "Not allowed: spawn workers, reply to workers, close workers, start watches, merge, push, open PRs, write to Linear, paid runs";

fn file_with(allowed: serde_json::Value) -> RunFile {
    let mut json = plan_json();
    json["allowed"] = allowed;
    RunFile {
        plan: parse_run(&json, limits()).unwrap(),
        approved: None,
        stopped: None,
    }
}

fn lines_of(text: &str, prefix: &str) -> Vec<String> {
    text.lines()
        .filter(|line| line.starts_with(prefix))
        .map(str::to_string)
        .collect()
}

fn plain_file() -> RunFile {
    RunFile {
        plan: plan(),
        approved: None,
        stopped: None,
    }
}

fn stopped_file() -> RunFile {
    RunFile {
        stopped: Some(Stopped {
            step: 2,
            hash: plan_hash(&plan()),
        }),
        ..plain_file()
    }
}

fn approved_file() -> RunFile {
    signed_go(&plain_file(), Some(&plan_hash(&plan())), true).unwrap()
}

#[test]
fn not_allowed_lists_every_action_the_plan_does_not_allow() {
    let text = render_run(&plan());
    assert_eq!(
        lines_of(&text, "Not allowed:"),
        vec![
            "Not allowed: reply to workers, close workers, start watches, push, open PRs, write to Linear"
        ]
    );
}

#[test]
fn a_plan_that_allows_nothing_denies_every_action() {
    let text = render_run(&file_with(serde_json::json!([])).plan);
    assert_eq!(lines_of(&text, "Not allowed:"), vec![EVERYTHING]);
    assert!(text.contains("\nAllowed: none\n"), "{text}");
}

#[test]
fn a_plan_that_allows_every_action_denies_none() {
    let all = serde_json::json!([
        {"kind": "spawn", "harness": "claude"}, {"kind": "reply"}, {"kind": "close"},
        {"kind": "watch"}, {"kind": "merge"}, {"kind": "push"}, {"kind": "pr"},
        {"kind": "linear_write"}, {"kind": "paid_run", "max_usd": 1.0}
    ]);
    let text = render_run(&file_with(all).plan);
    assert_eq!(lines_of(&text, "Not allowed:"), vec!["Not allowed: none"]);
}

#[test]
fn allowing_one_spawn_harness_removes_spawn_from_the_denied_list() {
    let text =
        render_run(&file_with(serde_json::json!([{"kind": "spawn", "harness": "claude"}])).plan);
    let denied = lines_of(&text, "Not allowed:").remove(0);
    assert!(!denied.contains("spawn"), "{denied}");
    assert!(denied.contains("merge"), "{denied}");
}

#[test]
fn the_warning_sits_between_not_allowed_and_caps() {
    let text = render_run(&file_with(serde_json::json!([{"kind": "merge"}])).plan);
    let lines: Vec<&str> = text.lines().collect();
    let at = |prefix: &str| {
        lines
            .iter()
            .position(|line| line.starts_with(prefix))
            .unwrap()
    };
    assert_eq!(at("Warning:"), at("Not allowed:") + 1, "{text}");
    assert_eq!(at("Caps:"), at("Warning:") + 1, "{text}");
}

#[test]
fn the_footer_is_one_status_line_and_always_last() {
    let hash = plan_hash(&plan());
    for (file, last) in [
        (
            plain_file(),
            format!("{hash} · not approved · /send-it go to approve"),
        ),
        (approved_file(), format!("{hash} · approved")),
        (
            stopped_file(),
            format!("{hash} · stopped at step 2/4 · /send-it go to resume from step 2"),
        ),
    ] {
        let status = render_status(&file, WIDE, TRUST);
        assert_eq!(status.lines().last(), Some(last.as_str()), "{status}");
        assert_eq!(status.matches(&hash).count(), 1, "{status}");
        let styled = render_status_styled(&file, WIDE, TRUST);
        assert_eq!(strip(&styled).lines().last(), Some(last.as_str()));
    }
}

#[test]
fn the_footer_is_last_after_the_warning_and_the_grants() {
    let mut file = file_with(serde_json::json!([]));
    file.plan.grants = vec![Grant::TurnCap { turns: 30 }];
    let status = render_status(&file, WIDE, TRUST);
    let lines: Vec<&str> = status.lines().collect();
    assert!(lines[lines.len() - 2].starts_with("  - raise"), "{status}");
    assert!(
        lines[lines.len() - 1].contains(" · not approved"),
        "{status}"
    );
}

#[test]
fn the_brief_has_no_status_footer() {
    let text = render_run(&plan());
    assert!(!text.contains("not approved"), "{text}");
    assert!(!text.contains(&plan_hash(&plan())), "{text}");
}

#[test]
fn the_styled_footer_dims_the_hash_and_colours_the_status() {
    let hash = plan_hash(&plan());
    let last = |file: &RunFile| {
        render_status_styled(file, WIDE, TRUST)
            .lines()
            .last()
            .unwrap()
            .to_string()
    };
    assert!(last(&plain_file()).starts_with(&format!("\x1b[2m{hash}\x1b[0m \x1b[33m")));
    assert!(last(&approved_file()).starts_with(&format!("\x1b[2m{hash}\x1b[0m \x1b[32m")));
    assert!(last(&stopped_file()).contains("\x1b[33m"));
}

#[test]
fn a_wrapped_footer_stays_the_last_lines_and_strips_to_the_plain_text() {
    let styled = render_status_styled(&stopped_file(), NARROW, TRUST);
    let plain = render_status(&stopped_file(), NARROW, TRUST);
    assert_eq!(strip(&styled), plain);
    assert!(plain.lines().count() > 1);
    assert!(
        plain.lines().all(|line| line.chars().count() <= NARROW),
        "{plain}"
    );
    assert!(plain.lines().last().unwrap().contains("step 2"), "{plain}");
}

#[test]
fn the_styled_warning_is_coloured() {
    let styled = render_status_styled(&file_with(serde_json::json!([])), WIDE, TRUST);
    let warning = styled
        .lines()
        .find(|line| line.contains("Warning:"))
        .unwrap();
    assert!(warning.starts_with("\x1b[33m"), "{warning:?}");
}
