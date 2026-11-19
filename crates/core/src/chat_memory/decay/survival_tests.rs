use super::ancestors::ancestors;
use super::anchors::{Anchor, Group};
use super::survival::{has_pointer, observe, pointer_cell, pointer_of, tally, Cell};
use super::tokens::{Class, Key};
use crate::chat_memory::fixtures::n;
use crate::chat_memory::Tree;

fn key(word: &str, class: Class) -> Key {
    Key {
        word: word.into(),
        class,
    }
}

fn anchor(id: usize, group: Group) -> Anchor {
    Anchor {
        id,
        group,
        keys: vec![
            key("guz-1637", Class::Ticket),
            key("split_view", Class::Identifier),
        ],
    }
}

fn tree() -> Tree {
    let mut tree = Tree::new();
    tree.insert(n(0, 2), "decided guz-1637 uses split_view #1".into());
    tree.insert(n(0, 4), "guz-1637 only #123".into());
    tree
}

#[test]
fn ancestors_list_aligned_blocks_with_summaries() {
    let tree = tree();
    let found = ancestors(1, 4, &tree);
    let nodes: Vec<(usize, usize, u32)> = found
        .iter()
        .map(|a| (a.node.lo, a.node.hi, a.depth))
        .collect();
    assert_eq!(nodes, [(0, 2, 1), (0, 4, 2)]);
    assert!(ancestors(3, 4, &tree).iter().all(|a| a.depth == 2));
}

#[test]
fn missing_summary_is_skipped() {
    let tree = Tree::new();
    assert!(ancestors(1, 4, &tree).is_empty());
    assert!(observe(&anchor(1, Group::Tagged), &[]).is_empty());
    assert_eq!(pointer_of(&anchor(1, Group::Tagged), &[]), None);
}

#[test]
fn survival_counts_by_depth_and_class() {
    let tree = tree();
    let found = ancestors(1, 4, &tree);
    let table = tally(&observe(&anchor(1, Group::Tagged), &found));
    let cell = |depth, class| table[&(depth, class)];
    assert_eq!(cell(1, Class::Ticket), Cell { kept: 1, total: 1 });
    assert_eq!(cell(1, Class::Identifier), Cell { kept: 1, total: 1 });
    assert_eq!(cell(2, Class::Ticket), Cell { kept: 1, total: 1 });
    assert_eq!(cell(2, Class::Identifier), Cell { kept: 0, total: 1 });
}

#[test]
fn pointer_found() {
    assert!(has_pointer("see #12 for more", 12));
    assert!(has_pointer("ends #12", 12));
}

#[test]
fn pointer_absent() {
    assert!(!has_pointer("nothing here", 12));
    assert!(!has_pointer("12 without hash", 12));
}

#[test]
fn pointer_ignores_block_ids() {
    assert!(!has_pointer("see #4-5", 4));
    assert!(has_pointer("see #4.", 4));
    assert!(has_pointer("see #4 ", 4));
    assert!(has_pointer("see #4-x", 4));
}

#[test]
fn pointer_ignores_longer_id() {
    assert!(!has_pointer("see #123", 12));
    assert!(has_pointer("see #123 and #12.", 12));
}

#[test]
fn groups_reported_separately() {
    let tree = tree();
    let found = ancestors(1, 4, &tree);
    let mut all = observe(&anchor(1, Group::Tagged), &found);
    all.extend(observe(&anchor(1, Group::TopUp), &ancestors(0, 4, &tree)));
    let of = |group| tally(all.iter().filter(|o| o.group == group));
    assert_eq!(of(Group::Tagged)[&(1, Class::Ticket)].total, 1);
    assert_eq!(of(Group::TopUp)[&(1, Class::Ticket)].total, 1);
    assert_eq!(of(Group::TopUp)[&(2, Class::Ticket)].total, 1);
    let pointers: Vec<_> = [
        pointer_of(&anchor(1, Group::Tagged), &found),
        pointer_of(&anchor(0, Group::TopUp), &ancestors(0, 4, &tree)),
    ]
    .into_iter()
    .flatten()
    .collect();
    let rate = |group| pointer_cell(pointers.iter().filter(|p| p.group == group));
    assert_eq!(rate(Group::Tagged), Cell { kept: 1, total: 1 });
    assert_eq!(rate(Group::TopUp), Cell { kept: 0, total: 1 });
}

#[test]
fn percent_rounds_and_handles_empty() {
    assert_eq!(Cell::default().percent(), None);
    assert_eq!(Cell { kept: 1, total: 3 }.percent(), Some(33));
    assert_eq!(Cell { kept: 2, total: 3 }.percent(), Some(67));
}
