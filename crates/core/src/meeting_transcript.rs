use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const HUMAN: &str = "human";
pub const EVERYONE: &str = "all";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptLine {
    pub from: String,
    pub to: String,
    pub text: String,
    pub at: DateTime<Utc>,
}

fn line(from: &str, to: &str, text: &str, at: DateTime<Utc>) -> TranscriptLine {
    TranscriptLine {
        from: from.to_string(),
        to: to.to_string(),
        text: text.to_string(),
        at,
    }
}

pub fn kickoff_line(first: &str, kickoff: &str, at: DateTime<Utc>) -> TranscriptLine {
    line(HUMAN, first, kickoff, at)
}

pub fn message_line(from: &str, to: &str, text: &str, at: DateTime<Utc>) -> TranscriptLine {
    line(from, to, text, at)
}

pub fn question_line(from: &str, question: &str, at: DateTime<Utc>) -> TranscriptLine {
    line(from, HUMAN, question, at)
}

pub fn answer_line(to: &str, answer: &str, at: DateTime<Utc>) -> TranscriptLine {
    line(HUMAN, to, answer, at)
}

pub fn end_line(from: &str, summary: &str, at: DateTime<Utc>) -> TranscriptLine {
    line(from, EVERYONE, summary, at)
}

pub fn to_jsonl(line: &TranscriptLine) -> Result<String, serde_json::Error> {
    let mut out = serde_json::to_string(line)?;
    out.push('\n');
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap()
    }

    #[test]
    fn lines_map_activity_to_endpoints() {
        let ends = |l: TranscriptLine| (l.from, l.to);
        assert_eq!(
            ends(kickoff_line("a", "go", at())),
            ("human".into(), "a".into())
        );
        assert_eq!(
            ends(message_line("a", "b", "hi", at())),
            ("a".into(), "b".into())
        );
        assert_eq!(
            ends(question_line("a", "why", at())),
            ("a".into(), "human".into())
        );
        assert_eq!(
            ends(answer_line("a", "because", at())),
            ("human".into(), "a".into())
        );
        assert_eq!(
            ends(end_line("a", "done", at())),
            ("a".into(), "all".into())
        );
    }

    #[test]
    fn jsonl_is_one_parseable_line_per_entry() {
        let original = message_line("a", "b", "multi\nline", at());
        let out = to_jsonl(&original).unwrap();
        assert_eq!(out.matches('\n').count(), 1);
        assert!(out.ends_with('\n'));
        let parsed: TranscriptLine = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed, original);
    }
}
