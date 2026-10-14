use super::*;

#[test]
fn only_worker_turns_log_before_the_reply() {
    assert!(logged_before_reply(Trigger::Worker));
    assert!(!logged_before_reply(Trigger::User));
}

#[test]
fn unanswered_turn_restores_the_draft() {
    assert_eq!(
        draft_after(TurnOutcome::Unanswered, "hi".into()),
        Some("hi".into())
    );
    assert_eq!(draft_after(TurnOutcome::Replied, "hi".into()), None);
    assert_eq!(draft_after(TurnOutcome::Exit, "hi".into()), None);
}

#[test]
fn rule_follows_shown_output_only() {
    assert!(!shown_output(&[]));
    assert!(shown_output(&[OutputBlock::Separator]));
}

#[test]
fn rule_spans_the_given_width() {
    let line = rule(&MadSkin::no_style(), 20);
    assert_eq!(line.trim_end().chars().count(), 20);
}

#[test]
fn second_esc_clears_the_draft() {
    let (pending, first) = esc_esc(false, ReedlineEvent::Esc);
    assert!(pending && first == ReedlineEvent::Esc);
    let (pending, second) = esc_esc(pending, ReedlineEvent::Esc);
    assert!(!pending && second == ReedlineEvent::Edit(vec![EditCommand::Clear]));
}

#[test]
fn other_event_resets_pending_esc() {
    let (pending, event) = esc_esc(true, ReedlineEvent::Enter);
    assert!(!pending && event == ReedlineEvent::Enter);
}
