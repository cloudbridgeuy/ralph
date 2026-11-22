use super::render::strip;
use super::*;

const REFUSED: &str = "Approval in run.toml refused: ";
const GO: &str = ". Type /send-it go to approve.";

fn old_format() -> RunFile {
    let unapproved = RunFile {
        approved: None,
        ..approved(plan())
    };
    let text = format!(
        "approved = \"{}\"\n{}",
        plan_hash(&plan()),
        write_run(&unapproved).unwrap()
    );
    read_run(&text, limits()).unwrap()
}

#[test]
fn each_refusal_says_why() {
    for (refusal, text) in [
        (Refusal::Unsigned, "not signed by this chat"),
        (
            Refusal::Tampered,
            "the signature does not match this chat (edited, or from before a restart)",
        ),
        (
            Refusal::WrongKind,
            "the record is not a /send-it go approval",
        ),
        (Refusal::WrongPlan, "the plan changed after it was approved"),
        (Refusal::Replayed, "the approval is no longer current"),
    ] {
        assert_eq!(refusal.to_string(), text);
    }
}

#[test]
fn no_record_and_a_good_record_have_no_refusal() {
    let none = RunFile {
        approved: None,
        ..approved(plan())
    };
    assert_eq!(approval_refusal(&none, TRUST), None);
    assert_eq!(approval_refusal(&approved(plan()), TRUST), None);
}

#[test]
fn a_record_from_before_a_restart_is_not_blamed_while_nothing_is_live() {
    let file = approved(plan());
    assert_eq!(
        approval_refusal(
            &file,
            Trust {
                live: None,
                ..TRUST
            }
        ),
        None
    );
    let other = Trust {
        live: Some("n2"),
        ..TRUST
    };
    assert_eq!(approval_refusal(&file, other), Some(Refusal::Replayed));
}

#[test]
fn an_old_string_is_unsigned() {
    assert_eq!(
        approval_refusal(&old_format(), TRUST),
        Some(Refusal::Unsigned)
    );
}

#[test]
fn a_flipped_mac_is_tampered_and_a_swapped_plan_is_wrong_plan() {
    let mut file = approved(plan());
    let approval = file.approved.as_mut().unwrap();
    let first = if approval.mac.starts_with('0') {
        "1"
    } else {
        "0"
    };
    approval.mac.replace_range(..1, first);
    assert_eq!(approval_refusal(&file, TRUST), Some(Refusal::Tampered));
    let mut swapped = approved(plan());
    swapped.plan.goal = "something else".to_string();
    assert_eq!(approval_refusal(&swapped, TRUST), Some(Refusal::WrongPlan));
}

#[test]
fn status_names_the_refusal_of_a_hand_written_approval() {
    let line = format!("{REFUSED}not signed by this chat{GO}");
    let file = old_format();
    for text in [
        render_status(&file, 200, TRUST),
        strip(&render_status_styled(&file, 200, TRUST)),
    ] {
        assert!(text.lines().any(|row| row == line), "{text}");
        assert!(text.contains("not approved"), "{text}");
    }
    assert!(!render_status(&approved(plan()), 200, TRUST).contains(REFUSED));
}

#[test]
fn a_tampered_record_mid_run_stops_with_the_reason() {
    let mut file = approved(plan());
    let mac = &mut file.approved.as_mut().unwrap().mac;
    let first = if mac.starts_with('0') { "1" } else { "0" };
    mac.replace_range(..1, first);
    let state = driving(0, Awaiting::Turn, Run::default());
    let next = tick(&state, &ended(&file)).next;
    assert_eq!(
        next,
        Next::Stop(StopReason::PlanChanged(Some(Refusal::Tampered)))
    );
    let Next::Stop(reason) = next else { return };
    let notice = reason.notice(&Run::default(), None);
    assert!(
        notice.contains("the signature does not match this chat"),
        "{notice}"
    );
}
