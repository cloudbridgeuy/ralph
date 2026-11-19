use super::anchors::{Anchor, Group};
use super::era::{Block, Era, Section};
use super::reach::{Bucket, ReachTally};
use super::render::render_decay;
use super::survival::Cell;
use super::tokens::Class;
use super::Decay;
use std::collections::BTreeMap;

fn anchor(id: usize, group: Group) -> Anchor {
    Anchor {
        id,
        group,
        keys: Vec::new(),
    }
}

fn sample_block() -> Block {
    Block {
        survival: BTreeMap::from([
            ((1, Class::Ticket), Cell { kept: 1, total: 1 }),
            ((1, Class::Identifier), Cell { kept: 1, total: 2 }),
            ((2, Class::Ticket), Cell { kept: 0, total: 1 }),
        ]),
        pointer: Cell { kept: 1, total: 3 },
        reach: ReachTally {
            unreachable: 1,
            search: BTreeMap::from([(Bucket::One, 1), (Bucket::Miss, 1)]),
            recall: BTreeMap::from([(Bucket::TwoToFive, 1), (Bucket::Miss, 1)]),
        },
    }
}

#[test]
fn render_sample_layout() {
    let tagged = Section {
        overall: sample_block(),
        eras: BTreeMap::from([(Era::Late, sample_block())]),
    };
    let decay = Decay {
        anchors: vec![
            anchor(1, Group::Tagged),
            anchor(2, Group::Tagged),
            anchor(3, Group::TopUp),
        ],
        uncompressed: 1,
        sections: BTreeMap::from([(Group::Tagged, tagged)]),
    };
    assert_eq!(
        render_decay(&decay),
        concat!(
            "anchors 3 (tagged 2, top-up 1) uncompressed 1\n",
            "\n",
            "== tagged (2 anchors) ==\n",
            "survival (key tokens kept in the summary) by depth:\n",
            "depth ticket      number      identifier  rare\n",
            "1     100% (1/1)  -           50% (1/2)   -\n",
            "2     0% (0/1)    -           -           -\n",
            "search rank: 1 1, 2-5 0, 6-20 0, >20 0, miss 1, unreachable 1\n",
            "recall found: 50% (1/2)\n",
            "pointer kept: 33% (1/3)\n",
            "by era (approximate: summaries are written after their entries):\n",
            "before 2026-11-12:\n",
            "  no data\n",
            "2026-11-12:\n",
            "  no data\n",
            "from 2026-11-13:\n",
            "  survival (key tokens kept in the summary) by depth:\n",
            "  depth ticket      number      identifier  rare\n",
            "  1     100% (1/1)  -           50% (1/2)   -\n",
            "  2     0% (0/1)    -           -           -\n",
            "  search rank: 1 1, 2-5 0, 6-20 0, >20 0, miss 1, unreachable 1\n",
            "  recall found: 50% (1/2)\n",
            "  pointer kept: 33% (1/3)\n",
            "\n",
            "== top-up (1 anchors) ==\n",
            "no data\n",
            "by era (approximate: summaries are written after their entries):\n",
            "before 2026-11-12:\n",
            "  no data\n",
            "2026-11-12:\n",
            "  no data\n",
            "from 2026-11-13:\n",
            "  no data",
        )
    );
}
