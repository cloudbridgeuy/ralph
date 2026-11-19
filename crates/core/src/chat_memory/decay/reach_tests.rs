use super::anchors::{Anchor, Group};
use super::reach::{
    bucket, cover_ancestor, probe, reach_tally, recall_position, search_rank, Bucket, Outcome,
};
use super::tokens::{Class, Key};
use crate::chat_memory::fixtures::{n, texted};
use crate::chat_memory::{Entry, Tree};

const TEXTS: [&str; 6] = [
    "alpha",
    "beta",
    "guz-1637 split_view rollout",
    "gamma",
    "split_view only here",
    "guz-1637 here",
];

fn entries() -> Vec<Entry> {
    texted(6, |i| TEXTS[i].to_string())
}

fn anchor(id: usize) -> Anchor {
    Anchor {
        id,
        group: Group::Tagged,
        keys: vec![
            Key {
                word: "guz-1637".into(),
                class: Class::Ticket,
            },
            Key {
                word: "split_view".into(),
                class: Class::Identifier,
            },
        ],
    }
}

fn tree(summary: &str) -> Tree {
    let mut tree = Tree::new();
    tree.insert(n(0, 4), summary.into());
    tree
}

#[test]
fn cover_ancestor_matches_cover() {
    let cover = [n(0, 4), n(4, 6), n(6, 7)];
    assert_eq!(cover_ancestor(5, &cover), Some(n(4, 6)));
    assert_eq!(cover_ancestor(0, &cover), Some(n(0, 4)));
    assert_eq!(cover_ancestor(9, &cover), None);
}

#[test]
fn leaf_in_cover_is_uncompressed() {
    let cover = [n(0, 4), n(4, 5), n(5, 6)];
    let found = probe(&anchor(5), &entries(), &tree("x"), &cover).unwrap();
    assert_eq!(found.outcome, Outcome::Uncompressed);
    assert_eq!(found.last, 5);
}

#[test]
fn search_rank_of_anchor() {
    let entries = entries();
    assert_eq!(
        search_rank(&entries, 2, &["guz-1637", "split_view"]),
        Some(1)
    );
    assert_eq!(search_rank(&entries, 0, &["guz-1637"]), None);
    let cover = [n(0, 4), n(4, 5), n(5, 6)];
    let found = probe(&anchor(2), &entries, &tree("guz-1637 split_view"), &cover).unwrap();
    assert_eq!(
        found.outcome,
        Outcome::Reached {
            search: Bucket::One,
            recall: Bucket::TwoToFive
        }
    );
}

#[test]
fn recall_position_or_miss() {
    let entries = entries();
    assert_eq!(
        recall_position(&entries, 2, &["guz-1637", "split_view"]),
        Some(3)
    );
    assert_eq!(
        recall_position(&entries, 5, &["guz-1637", "split_view"]),
        Some(1)
    );
    assert_eq!(recall_position(&entries, 0, &["guz-1637"]), None);
}

#[test]
fn no_tokens_is_unreachable() {
    let cover = [n(0, 4), n(4, 5), n(5, 6)];
    let found = probe(&anchor(2), &entries(), &tree("nothing kept"), &cover).unwrap();
    assert_eq!(found.outcome, Outcome::Unreachable);
    let missing = probe(&anchor(2), &entries(), &Tree::new(), &cover).unwrap();
    assert_eq!(missing.outcome, Outcome::Unreachable);
}

#[test]
fn rank_buckets() {
    let buckets: Vec<Bucket> = [Some(1), Some(2), Some(5), Some(6), Some(20), Some(21), None]
        .into_iter()
        .map(bucket)
        .collect();
    assert_eq!(
        buckets,
        [
            Bucket::One,
            Bucket::TwoToFive,
            Bucket::TwoToFive,
            Bucket::SixToTwenty,
            Bucket::SixToTwenty,
            Bucket::Over,
            Bucket::Miss
        ]
    );
}

#[test]
fn reach_tally_counts_outcomes_and_skips_uncompressed() {
    let cover = [n(0, 4), n(4, 5), n(5, 6)];
    let entries = entries();
    let tree = tree("guz-1637 split_view");
    let probes: Vec<_> = [2, 5, 0]
        .iter()
        .map(|id| probe(&anchor(*id), &entries, &tree, &cover).unwrap())
        .collect();
    let tally = reach_tally(&probes);
    assert_eq!(tally.search.values().sum::<usize>(), 2);
    assert_eq!(tally.recall.values().sum::<usize>(), 2);
    assert_eq!(tally.recall[&Bucket::Miss], 1);
}
