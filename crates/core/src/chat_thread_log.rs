use crate::chat_thread::{Merged, ThreadMeta, ThreadName};
use serde::{Deserialize, Serialize};

pub const DROPPED_FILE: &str = "dropped.jsonl";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tombstone {
    pub name: String,
    pub base: usize,
    pub created: String,
    pub dropped_at: String,
}

pub fn tombstone_line(name: &ThreadName, meta: &ThreadMeta, dropped_at: &str) -> String {
    let tombstone = Tombstone {
        name: name.to_string(),
        base: meta.base,
        created: meta.created.clone(),
        dropped_at: dropped_at.to_string(),
    };
    let mut line = serde_json::to_string(&tombstone).unwrap_or_default();
    if !line.is_empty() {
        line.push('\n');
    }
    line
}

pub fn parse_tombstones(text: &str) -> Vec<Tombstone> {
    text.lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

pub fn with_landing(meta: &ThreadMeta, id: usize) -> Option<ThreadMeta> {
    let merged = meta
        .merged
        .as_ref()
        .filter(|merged| merged.landed.is_none())?;
    Some(ThreadMeta {
        merged: Some(Merged {
            landed: Some(id),
            ..merged.clone()
        }),
        ..meta.clone()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn merged(landed: Option<usize>) -> ThreadMeta {
        ThreadMeta {
            merged: Some(Merged {
                at: "t".to_string(),
                digest: "d".to_string(),
                landed,
            }),
            ..meta(3)
        }
    }

    #[test]
    fn a_merged_thread_without_a_landing_takes_the_id() {
        let patched = with_landing(&merged(None), 9).unwrap();
        assert_eq!(patched, merged(Some(9)));
    }

    #[test]
    fn an_unmerged_thread_is_left_alone() {
        assert_eq!(with_landing(&meta(3), 9), None);
    }

    #[test]
    fn a_known_landing_is_never_replaced() {
        assert_eq!(with_landing(&merged(Some(4)), 9), None);
    }

    #[test]
    fn a_thread_file_without_landed_reads_as_unknown() {
        let old = r#"{"base":3,"created":"2026-01-01T00:00:00Z","merged":{"at":"t","digest":"d"}}"#;
        let meta: ThreadMeta = serde_json::from_str(old).unwrap();
        assert_eq!(meta, merged(None));
    }

    #[test]
    fn landed_round_trips_and_unknown_is_not_written() {
        let known = serde_json::to_string(&merged(Some(12))).unwrap();
        assert!(known.contains(r#""landed":12"#));
        assert_eq!(
            serde_json::from_str::<ThreadMeta>(&known).unwrap(),
            merged(Some(12))
        );
        let unknown = serde_json::to_string(&merged(None)).unwrap();
        assert!(!unknown.contains("landed"));
    }

    fn meta(base: usize) -> ThreadMeta {
        ThreadMeta {
            base,
            created: "2026-01-01T00:00:00Z".to_string(),
            merged: None,
            parked: None,
        }
    }

    fn line(name: &str, base: usize) -> String {
        tombstone_line(
            &ThreadName::parse(name).unwrap(),
            &meta(base),
            "2026-02-02T00:00:00Z",
        )
    }

    #[test]
    fn round_trips_one_tombstone() {
        let parsed = parse_tombstones(&line("alpha", 7));
        assert_eq!(
            parsed,
            vec![Tombstone {
                name: "alpha".to_string(),
                base: 7,
                created: "2026-01-01T00:00:00Z".to_string(),
                dropped_at: "2026-02-02T00:00:00Z".to_string(),
            }]
        );
    }

    #[test]
    fn line_is_one_newline_terminated_row() {
        let text = line("alpha", 7);
        assert!(text.ends_with('\n'));
        assert_eq!(text.matches('\n').count(), 1);
    }

    #[test]
    fn skips_blank_and_bad_lines() {
        let text = format!(
            "\n{}not json\n{{\"name\":1}}\n{}",
            line("a", 1),
            line("b", 2)
        );
        let names: Vec<_> = parse_tombstones(&text)
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn keeps_file_order() {
        let text = format!("{}{}{}", line("z", 5), line("a", 1), line("m", 3));
        let names: Vec<_> = parse_tombstones(&text)
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, vec!["z", "a", "m"]);
    }
}
