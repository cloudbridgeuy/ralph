use super::anchors::{pick_anchors, Group};
use super::tokens::{document_frequency, Class};
use crate::chat_memory::fixtures::texted;
use crate::chat_memory::tag::Flags;
use crate::chat_memory::{decode_rows, encode_row, Entry, RowKind, TagRow, TagState};

fn row(n: usize, kind: RowKind) -> String {
    encode_row(&TagRow {
        n,
        kind,
        model: "m".into(),
        prompt: "v".into(),
        at: "2026-11-01T00:00:00Z".into(),
    })
    .unwrap()
}

fn flags(rule: bool, decision: bool) -> RowKind {
    RowKind::Tagged(Flags {
        rule,
        decision,
        approval: false,
    })
}

fn state(rows: &[String]) -> TagState {
    decode_rows(rows.concat().as_bytes())
}

fn pick(entries: &[Entry], tags: &TagState, max: usize) -> Vec<(usize, Group)> {
    let frequency = document_frequency(entries);
    pick_anchors(entries, tags, &frequency, max)
        .into_iter()
        .map(|a| (a.id, a.group))
        .collect()
}

#[test]
fn pick_tagged_rule_and_decision_first() {
    let entries = texted(6, |i| format!("note {i} about split_view"));
    let tags = state(&[
        row(4, flags(true, false)),
        row(1, flags(false, true)),
        row(2, flags(false, false)),
    ]);
    let picked = pick(&entries, &tags, 10);
    assert_eq!(
        picked,
        [
            (1, Group::Tagged),
            (4, Group::Tagged),
            (0, Group::TopUp),
            (2, Group::TopUp),
        ]
    );
}

#[test]
fn skip_failed_and_skipped_rows() {
    let entries = texted(4, |_| "keep split_view".to_string());
    let tags = state(&[
        row(0, RowKind::Failed),
        row(1, RowKind::Skipped),
        row(2, flags(true, false)),
    ]);
    let tagged: Vec<usize> = pick(&entries, &tags, 10)
        .into_iter()
        .filter(|(_, g)| *g == Group::Tagged)
        .map(|(id, _)| id)
        .collect();
    assert_eq!(tagged, [2]);
}

#[test]
fn topup_takes_untagged_user_entries_with_ticket_or_identifier() {
    let texts = [
        "see guz-1637",
        "plain words only",
        "call split_view now",
        "assistant says guz-1637",
        "number 4096 only",
    ];
    let mut entries = texted(5, |i| texts[i].to_string());
    for (i, entry) in entries.iter_mut().enumerate() {
        entry.role = if i == 3 {
            crate::chat_memory::Role::Assistant
        } else {
            crate::chat_memory::Role::User
        };
    }
    let picked = pick(&entries, &TagState::default(), 10);
    assert_eq!(picked, [(0, Group::TopUp), (2, Group::TopUp)]);
}

#[test]
fn cap_keeps_tagged_first() {
    let entries = texted(6, |_| "split_view".to_string());
    let tags = state(&[row(5, flags(true, false))]);
    assert_eq!(
        pick(&entries, &tags, 2),
        [(5, Group::Tagged), (0, Group::TopUp)]
    );
}

#[test]
fn anchors_carry_their_key_tokens() {
    let entries = texted(1, |_| "go guz-1637 on 42".to_string());
    let frequency = document_frequency(&entries);
    let anchors = pick_anchors(&entries, &TagState::default(), &frequency, 5);
    let classes: Vec<Class> = anchors[0].keys.iter().map(|k| k.class).collect();
    assert_eq!(classes, [Class::Ticket, Class::Number]);
}
