use super::anchors::Group;
use super::era::{era_of, section, Collected, Era};
use super::survival::{Observation, Pointer};
use super::tokens::Class;
use crate::chat_memory::fixtures::texted;

fn obs(group: Group, last: usize, kept: bool) -> Observation {
    Observation {
        group,
        depth: 1,
        class: Class::Ticket,
        kept,
        last,
    }
}

#[test]
fn era_buckets_by_entry_date() {
    assert_eq!(era_of("2026-11-11T23:59:59Z"), Era::Early);
    assert_eq!(era_of("2026-11-12T00:00:00Z"), Era::Mid);
    assert_eq!(era_of("2026-11-12T23:59:59Z"), Era::Mid);
    assert_eq!(era_of("2026-11-13T00:00:00Z"), Era::Late);
    assert_eq!(era_of("2027-01-01T00:00:00Z"), Era::Late);
    assert_eq!(era_of(""), Era::Early);
}

#[test]
fn section_splits_by_the_node_last_entry_date() {
    let mut entries = texted(3, |_| "x".to_string());
    entries[0].at = "2026-11-01T00:00:00Z".into();
    entries[1].at = "2026-11-12T10:00:00Z".into();
    entries[2].at = "2026-11-14T10:00:00Z".into();
    let observations = [
        obs(Group::Tagged, 0, true),
        obs(Group::Tagged, 1, false),
        obs(Group::Tagged, 2, true),
        obs(Group::TopUp, 2, true),
    ];
    let pointers = [Pointer {
        group: Group::Tagged,
        found: true,
        last: 2,
    }];
    let data = Collected {
        observations: &observations,
        pointers: &pointers,
        probes: &[],
    };
    let tagged = section(Group::Tagged, &data, &entries);
    let total = |era| tagged.eras[&era].survival[&(1, Class::Ticket)].total;
    assert_eq!(tagged.overall.survival[&(1, Class::Ticket)].total, 3);
    assert_eq!(
        (total(Era::Early), total(Era::Mid), total(Era::Late)),
        (1, 1, 1)
    );
    assert_eq!(tagged.eras[&Era::Late].pointer.kept, 1);
    assert!(!tagged.eras[&Era::Mid].is_empty());
    assert!(section(Group::TopUp, &data, &entries).eras[&Era::Early].is_empty());
}

#[test]
fn build_decay_counts_keyless_anchors_per_group() {
    use super::{build_decay, DecayConfig};
    use crate::chat_memory::{decode_rows, encode_row, tag::Flags, RowKind, TagRow, Tree};
    let entries = texted(4, |i| {
        if i == 0 {
            "go on".into()
        } else {
            "see split_view".into()
        }
    });
    let row = encode_row(&TagRow {
        n: 0,
        kind: RowKind::Tagged(Flags {
            rule: true,
            decision: false,
            approval: false,
        }),
        model: "m".into(),
        prompt: "v".into(),
        at: "2026-11-01T00:00:00Z".into(),
    })
    .unwrap();
    let tags = decode_rows(row.as_bytes());
    let decay = build_decay(&entries, &Tree::new(), &tags, &DecayConfig::default());
    assert_eq!(decay.keyless.get(&Group::Tagged), Some(&1));
    assert_eq!(decay.keyless.get(&Group::TopUp).copied().unwrap_or(0), 0);
}
