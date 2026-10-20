use super::fixtures::{entry, n};
use super::{pending, Entry, Node, Tree};

fn long_log(count: usize, bad: usize) -> Vec<Entry> {
    (0..count)
        .map(|i| {
            if i == bad {
                Entry::corrupt()
            } else {
                entry(i, 600)
            }
        })
        .collect()
}

fn offered(count: usize, bad: usize) -> Vec<Node> {
    pending(&long_log(count, bad), &Tree::new())
}

fn assert_split(count: usize, bad: usize, skipped: &[Node], kept: &[Node]) {
    let nodes = offered(count, bad);
    for node in skipped {
        assert!(!nodes.contains(node), "{} offered for bad {bad}", node.id());
    }
    for node in kept {
        assert!(nodes.contains(node), "{} missing for bad {bad}", node.id());
    }
    assert!(nodes.iter().all(|node| !(node.lo..node.hi).contains(&bad)));
}

#[test]
fn a_placeholder_at_the_first_entry_of_a_block_skips_every_range_holding_it() {
    assert_split(
        8,
        4,
        &[n(4, 5), n(4, 6), n(4, 8), n(0, 8)],
        &[n(3, 4), n(5, 6), n(0, 4), n(6, 8), n(0, 2), n(2, 4)],
    );
}

#[test]
fn a_placeholder_at_the_last_entry_of_a_block_skips_every_range_holding_it() {
    assert_split(
        8,
        7,
        &[n(7, 8), n(6, 8), n(4, 8), n(0, 8)],
        &[n(6, 7), n(0, 4), n(4, 6), n(0, 2), n(2, 4)],
    );
    assert_split(
        8,
        3,
        &[n(3, 4), n(2, 4), n(0, 4), n(0, 8)],
        &[n(2, 3), n(0, 2), n(4, 8), n(4, 6), n(6, 8)],
    );
}

#[test]
fn a_placeholder_at_either_end_of_the_log_skips_only_ranges_holding_it() {
    assert_split(
        8,
        0,
        &[n(0, 1), n(0, 2), n(0, 4), n(0, 8)],
        &[n(1, 2), n(2, 4), n(4, 8), n(6, 8)],
    );
    assert_eq!(offered(8, 7).len(), 11);
    assert_eq!(offered(8, 4).len(), 11);
}
