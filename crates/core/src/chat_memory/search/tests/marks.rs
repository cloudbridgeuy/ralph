use super::{archive, user};
use crate::chat_memory::{parse_search_args, search_found, Flags, Marks};
use serde_json::json;

fn flags(rule: bool, decision: bool, approval: bool) -> Flags {
    Flags {
        rule,
        decision,
        approval,
    }
}

fn run(marks: &Marks) -> Vec<String> {
    let entries = [user(0, "needle zero"), user(1, "needle one")];
    let request = parse_search_args(&json!({"query": "needle"})).unwrap();
    search_found(&mut archive(&entries), &request, marks)
        .unwrap()
        .unwrap()
        .text
        .lines()
        .filter(|line| line.starts_with('#'))
        .map(str::to_string)
        .collect()
}

#[test]
fn tagged_hits_carry_the_mark_after_the_role() {
    let marks = Marks::from([(1, flags(true, false, true))]);
    let lines = run(&marks);
    assert!(
        lines.contains(
            &"#1 · user [rule,approval] · 2026-10-20T12:01:00.000000000+00:00 · needle one"
                .to_string()
        ),
        "{lines:?}"
    );
    assert!(
        lines
            .contains(&"#0 · user · 2026-10-20T12:00:00.000000000+00:00 · needle zero".to_string()),
        "{lines:?}"
    );
}

#[test]
fn untagged_hits_are_unchanged() {
    assert_eq!(
        run(&Marks::new()),
        run(&Marks::from([(0, flags(false, false, false))]))
    );
    assert!(run(&Marks::new()).iter().all(|line| !line.contains('[')));
}
