use super::stamp;
use ralph_core::chat_memory::{encode_entry, encode_record, NewEntry, Node, Role};

const TOPICS: [&str; 20] = [
    "tea brewing times",
    "garden planning",
    "a bicycle gear table",
    "bread starter schedule",
    "a reading list",
    "a packing checklist",
    "a chess opening note",
    "a recipe for lentil soup",
    "a plan for a quiet weekend",
    "a map of local parks",
    "a birdwatching log",
    "a jar of pickles",
    "a study timetable",
    "a list of board games",
    "a plan for a small shelf",
    "a note on cloud shapes",
    "a walking route by the river",
    "a list of herbs",
    "a plan for a picnic",
    "a sketch of a paper boat",
];

const SUMMARY_RANGES: [(usize, usize); 6] =
    [(0, 8), (8, 16), (16, 24), (24, 32), (0, 16), (16, 32)];

pub fn memory_entries() -> Vec<NewEntry> {
    TOPICS
        .iter()
        .zip(0_i64..)
        .flat_map(|(topic, index)| {
            let asked = -(20 - index) * 3_600;
            [
                NewEntry {
                    role: Role::User,
                    at: stamp(asked),
                    text: format!("Demo question {}: tell me about {topic}.", index + 1),
                },
                NewEntry {
                    role: Role::Assistant,
                    at: stamp(asked + 60),
                    text: format!(
                        "Demo answer {}: here is a short made-up note about {topic}.",
                        index + 1
                    ),
                },
            ]
        })
        .collect()
}

pub fn memory_summaries() -> Result<Vec<(Node, String)>, String> {
    SUMMARY_RANGES
        .iter()
        .map(|&(lo, hi)| {
            let text = format!(
                "Demo summary of entries {lo} to {}: short made-up notes on {} and {}.",
                hi - 1,
                TOPICS[lo / 2],
                TOPICS[(hi - 1) / 2]
            );
            Node::new(lo, hi).map(|node| (node, text))
        })
        .collect()
}

pub fn encode_log(entries: &[NewEntry]) -> Result<String, String> {
    entries.iter().map(encode_entry).collect()
}

pub fn encode_tree(nodes: &[(Node, String)]) -> Result<String, String> {
    nodes
        .iter()
        .map(|(node, text)| encode_record(*node, text))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn memory_has_about_twenty_turns_that_alternate() {
        let entries = memory_entries();
        assert_eq!(entries.len(), 40);
        for (index, entry) in entries.iter().enumerate() {
            let expected = if index % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            };
            assert_eq!(entry.role, expected);
            assert!(!entry.text.is_empty());
        }
        let times: Vec<&String> = entries.iter().map(|entry| &entry.at).collect();
        let mut sorted = times.clone();
        sorted.sort();
        assert_eq!(times, sorted);
    }

    #[test]
    fn memory_summaries_are_valid_blocks_inside_the_log() {
        let summaries = memory_summaries().unwrap();
        assert!(!summaries.is_empty());
        let total = memory_entries().len();
        let unique: HashSet<Node> = summaries.iter().map(|(node, _)| *node).collect();
        assert_eq!(unique.len(), summaries.len());
        for (node, text) in &summaries {
            assert!(node.hi <= total);
            assert!(!text.is_empty());
        }
    }

    #[test]
    fn the_encoded_log_and_tree_read_back() {
        let entries = memory_entries();
        let (parsed, warnings) =
            ralph_core::chat_memory::parse_log(encode_log(&entries).unwrap().as_bytes());
        assert!(warnings.is_empty());
        assert_eq!(parsed.len(), entries.len());
        assert_eq!(parsed[3].text, entries[3].text);
        let summaries = memory_summaries().unwrap();
        let (tree, warnings) =
            ralph_core::chat_memory::parse_tree(encode_tree(&summaries).unwrap().as_bytes());
        assert!(warnings.is_empty());
        assert_eq!(tree.len(), summaries.len());
    }
}
