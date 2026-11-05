use super::*;
use crate::chat_send_it::template::*;

fn ticketed() -> RunPlan {
    let mut plan = plan();
    plan.goal = "ship GUZ-12".to_string();
    plan.steps[0].task = "review GUZ-12, not GUZ-123 or GUZ-12-old, on worker/GUZ-12".to_string();
    plan.steps[2].criterion = Criterion::CheckPasses {
        command: "gh pr view worker/GUZ-12".to_string(),
    };
    plan
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| name.to_string()).collect()
}

#[test]
fn save_and_templates_parse_as_keywords_with_one_or_two_words() {
    let save = |ticket: Option<&str>| SendItCommand::Save {
        name: "ship".to_string(),
        ticket: ticket.map(str::to_string),
    };
    assert_eq!(parse_send_it_args(" templates "), SendItCommand::Templates);
    assert_eq!(parse_send_it_args("save ship"), save(None));
    assert_eq!(
        parse_send_it_args("save  ship  GUZ-9 "),
        save(Some("GUZ-9"))
    );
    for text in ["save", "save the plan for later", "templates please"] {
        assert_eq!(
            parse_send_it_args(text),
            SendItCommand::Propose(text.to_string())
        );
    }
}

#[test]
fn a_template_name_takes_only_letters_digits_and_dashes() {
    assert_eq!(template_name("ship-it-2"), Ok("ship-it-2"));
    for bad in ["", "-", "../x", "a/b", "a.toml", "a b"] {
        assert!(template_name(bad).is_err(), "{bad}");
    }
}

#[test]
fn saving_with_a_ticket_swaps_only_the_whole_ticket_for_the_placeholder() {
    let text = template_text(&ticketed(), Some("GUZ-12")).unwrap();
    assert!(!text.contains("approved"), "{text}");
    assert!(text.contains("goal = \"ship {ticket}\""), "{text}");
    assert!(
        text.contains("review {ticket}, not GUZ-123 or GUZ-12-old, on worker/{ticket}"),
        "{text}"
    );
    assert!(text.contains("gh pr view worker/{ticket}"), "{text}");
    assert!(template_text(&ticketed(), None)
        .unwrap()
        .contains("ship GUZ-12"));
    assert!(template_text(&ticketed(), Some("$(rm -rf ~)")).is_err());
}

#[test]
fn a_filled_template_is_the_plan_for_the_new_ticket_and_unapproved() {
    let text = template_text(&ticketed(), Some("GUZ-12")).unwrap();
    let filled = fill_template(&text, "ABC-7", limits()).unwrap();
    assert_eq!(
        filled.plan.steps[0].task,
        "review ABC-7, not GUZ-123 or GUZ-12-old, on worker/ABC-7"
    );
    assert_eq!(filled.approved, None);
    let stopped = format!("{text}\n[stopped]\nstep = 2\nhash = \"abc\"\n");
    assert_eq!(
        fill_template(&stopped, "ABC-7", limits()).unwrap().stopped,
        None
    );
    for bad in ["", "a b", "x\"y", "$(id)"] {
        assert!(fill_template(&text, bad, limits()).is_err(), "{bad}");
    }
    assert!(fill_template("goal = ", "GUZ-1", limits())
        .unwrap_err()
        .contains("malformed"));
}

#[test]
fn the_save_notice_says_whether_the_ticket_became_the_placeholder() {
    let swapped = template_text(&ticketed(), Some("GUZ-12")).unwrap();
    let literal = template_text(&ticketed(), None).unwrap();
    assert_eq!(
        save_notice("ship", Some("GUZ-12"), &swapped, false),
        "Template ship saved: GUZ-12 became {ticket}. Use it with /send-it <ticket> ship."
    );
    assert_eq!(
        save_notice("ship", None, &literal, true),
        "Template ship replaced. No ticket given: the plan is saved as is. Use it with /send-it <ticket> ship."
    );
    assert_eq!(
        save_notice("ship", Some("GUZ-77"), &literal, false),
        "Template ship saved: GUZ-77 is not in the plan, so it is saved as is. Use it with /send-it <ticket> ship."
    );
}

#[test]
fn only_a_ticket_and_an_exact_template_name_use_a_template() {
    let known = names(&["ship", "review"]);
    assert_eq!(
        match_template(" GUZ-9  ship ", &known),
        Some(("GUZ-9".to_string(), "ship".to_string()))
    );
    for text in ["GUZ-9 Ship", "ship GUZ-9", "GUZ-9 ship now", "ship"] {
        assert_eq!(match_template(text, &known), None, "{text}");
    }
}

#[test]
fn the_template_list_names_each_template_or_says_how_to_save_one() {
    assert_eq!(
        render_templates(&[]),
        "No templates. Save the stored plan with /send-it save <name> [<ticket>]."
    );
    assert_eq!(
        render_templates(&names(&["review", "ship"])),
        "Templates: review, ship. Use one with /send-it <ticket> <name>."
    );
}
