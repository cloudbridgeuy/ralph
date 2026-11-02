use super::error::{ReleaseError, Result};
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub subject: String,
    pub ids: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ticket {
    pub identifier: String,
    pub title: String,
    pub labels: Vec<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Notes {
    pub features: Vec<String>,
    pub improvements: Vec<String>,
    pub fixes: Vec<String>,
    pub other: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Features,
    Improvements,
    Fixes,
}

const LABEL_SECTIONS: [(&str, Section); 3] = [
    ("Feature", Section::Features),
    ("Improvement", Section::Improvements),
    ("Bug", Section::Fixes),
];

fn extract_ids(text: &str) -> Vec<u32> {
    let mut ids = Vec::new();
    for (start, marker) in text.match_indices("GUZ-") {
        let digits: String = text[start + marker.len()..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if let Ok(id) = digits.parse() {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    ids
}

pub fn parse_log(raw: &str) -> Vec<Commit> {
    raw.split('\x1e')
        .filter_map(|record| {
            let mut fields = record.trim_start_matches('\n').splitn(3, '\x1f');
            let _hash = fields.next()?;
            let subject = fields.next()?.trim().to_string();
            let body = fields.next().unwrap_or_default();
            let ids = extract_ids(&format!("{subject}\n{body}"));
            Some(Commit { subject, ids })
        })
        .collect()
}

fn parse_ticket(value: &serde_json::Value) -> Option<Ticket> {
    let labels = value["labels"]["nodes"]
        .as_array()
        .map(|nodes| {
            nodes
                .iter()
                .filter_map(|node| node["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Some(Ticket {
        identifier: value["identifier"].as_str()?.to_string(),
        title: value["title"].as_str()?.to_string(),
        labels,
    })
}

pub fn parse_linear(json: &str) -> Result<HashMap<u32, Ticket>> {
    let value: serde_json::Value = serde_json::from_str(json)?;
    let data = value["data"].as_object().ok_or(ReleaseError::MissingData)?;
    Ok(data
        .iter()
        .filter_map(|(alias, ticket)| {
            let id = alias.strip_prefix('t')?.parse().ok()?;
            Some((id, parse_ticket(ticket)?))
        })
        .collect())
}

pub fn linear_query(ids: &[u32]) -> String {
    let fields = ids
        .iter()
        .map(|id| {
            format!(
                "t{id}: issue(id: \"GUZ-{id}\") {{ identifier title labels {{ nodes {{ name }} }} }}"
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!("query {{ {fields} }}")
}

fn conventional_type(subject: &str) -> Option<&str> {
    let (head, _) = subject.split_once(':')?;
    let head = head.strip_suffix('!').unwrap_or(head);
    let kind = head.split_once('(').map_or(head, |(kind, _)| kind);
    (!kind.is_empty() && kind.chars().all(|c| c.is_ascii_alphabetic())).then_some(kind)
}

fn section_from_labels(labels: &[String]) -> Option<Section> {
    LABEL_SECTIONS
        .iter()
        .find(|(name, _)| labels.iter().any(|label| label == name))
        .map(|(_, section)| *section)
}

fn section_from_subject(subject: &str) -> Section {
    match conventional_type(subject) {
        Some("feat") => Section::Features,
        Some("fix") => Section::Fixes,
        _ => Section::Improvements,
    }
}

fn strip_id_suffix(subject: &str, id: u32) -> &str {
    subject
        .strip_suffix(&format!("(GUZ-{id})"))
        .map_or(subject, str::trim_end)
}

pub fn build_notes(commits: &[Commit], tickets: &HashMap<u32, Ticket>) -> Notes {
    let mut first_subject: BTreeMap<u32, &str> = BTreeMap::new();
    let mut unticketed: Vec<&str> = Vec::new();
    for commit in commits {
        if commit.ids.is_empty() {
            unticketed.push(&commit.subject);
        }
        for id in &commit.ids {
            first_subject.entry(*id).or_insert(&commit.subject);
        }
    }

    let mut notes = Notes::default();
    for (id, subject) in first_subject {
        let ticket = tickets.get(&id);
        let section = ticket
            .and_then(|ticket| section_from_labels(&ticket.labels))
            .unwrap_or_else(|| section_from_subject(subject));
        let title = ticket.map_or_else(|| strip_id_suffix(subject, id), |t| t.title.as_str());
        let entry = format!("- {title} (GUZ-{id})");
        match section {
            Section::Features => notes.features.push(entry),
            Section::Improvements => notes.improvements.push(entry),
            Section::Fixes => notes.fixes.push(entry),
        }
    }

    let (listed, rest): (Vec<&str>, Vec<&str>) = unticketed
        .into_iter()
        .partition(|subject| matches!(conventional_type(subject), Some("feat" | "fix")));
    notes.other = listed
        .iter()
        .map(|subject| format!("- {subject}"))
        .collect();
    if !rest.is_empty() {
        notes
            .other
            .push(format!("- plus {} other commits", rest.len()));
    }
    notes
}

pub fn render_section(version: &str, date: &str, summary: Option<&str>, notes: &Notes) -> String {
    let mut out = format!("## {version} - {date}\n");
    if let Some(summary) = summary.map(str::trim).filter(|s| !s.is_empty()) {
        out.push_str(&format!("\n{summary}\n"));
    }
    for (heading, entries) in [
        ("Features", &notes.features),
        ("Improvements", &notes.improvements),
        ("Fixes", &notes.fixes),
        ("Other", &notes.other),
    ] {
        if !entries.is_empty() {
            out.push_str(&format!("\n### {heading}\n\n{}\n", entries.join("\n")));
        }
    }
    out
}

pub fn prepend_changelog(existing: Option<&str>, section: &str) -> String {
    let previous = existing
        .map(|text| text.strip_prefix("# Changelog").unwrap_or(text).trim())
        .filter(|text| !text.is_empty());
    let section = section.trim_end();
    match previous {
        Some(previous) => format!("# Changelog\n\n{section}\n\n{previous}\n"),
        None => format!("# Changelog\n\n{section}\n"),
    }
}

pub fn summary_prompt(list: &str) -> String {
    format!(
        "Write a two to three sentence summary of this release for its changelog. Plain prose, no heading, no bullet list, no preamble. Mention only what the list supports. Output only the summary text. Do not add questions, next steps, instructions or offers after it.\n\n{list}"
    )
}

const CHATTER_PREFIXES: [&str; 3] = ["next:", "note:", "let me know"];
const CHATTER_PHRASES: [&str; 2] = ["paste this", "changelog entry above"];
const PREAMBLE_PREFIXES: [&str; 4] = ["here is", "here's", "sure", "summary"];
const MAX_QUESTION_WORDS: usize = 8;
const MAX_PREAMBLE_WORDS: usize = 6;

fn is_preamble(line: &str) -> bool {
    let lower = line.trim().to_lowercase();
    lower.ends_with(':')
        && lower.split_whitespace().count() <= MAX_PREAMBLE_WORDS
        && PREAMBLE_PREFIXES
            .iter()
            .any(|prefix| lower.starts_with(prefix))
}

/// A trailing line the model added after the summary. A short question only
/// counts when it stands as its own paragraph, so a wrapped summary sentence
/// that ends in a question mark is kept.
fn is_chatter(line: &str, own_paragraph: bool) -> bool {
    let lower = line.trim().to_lowercase();
    let short_question =
        lower.ends_with('?') && lower.split_whitespace().count() <= MAX_QUESTION_WORDS;
    CHATTER_PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(prefix))
        || CHATTER_PHRASES.iter().any(|phrase| lower.contains(phrase))
        || (own_paragraph && short_question)
}

fn trim_blank_edges(lines: &mut Vec<&str>) {
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    let leading = lines
        .iter()
        .take_while(|line| line.trim().is_empty())
        .count();
    lines.drain(..leading);
}

/// Strip model chatter from a generated summary: code fences, a leading
/// "here's the summary:" line, and trailing next-step lines or offers.
/// Trailing lines are only dropped after the first blank line, so the first
/// paragraph always survives.
fn clean_summary(raw: &str) -> Option<String> {
    let mut lines: Vec<&str> = raw
        .lines()
        .filter(|line| !line.trim_start().starts_with("```"))
        .collect();
    trim_blank_edges(&mut lines);
    if lines.first().is_some_and(|line| is_preamble(line)) {
        lines.remove(0);
        trim_blank_edges(&mut lines);
    }
    let first_break = lines
        .iter()
        .position(|line| line.trim().is_empty())
        .unwrap_or(lines.len());
    while lines.len() > first_break + 1 {
        let Some((last, before)) = lines.split_last() else {
            break;
        };
        let own_paragraph = before.last().is_none_or(|line| line.trim().is_empty());
        if !last.trim().is_empty() && !is_chatter(last, own_paragraph) {
            break;
        }
        lines.pop();
    }
    let text = lines.join("\n").trim().to_string();
    (!text.is_empty()).then_some(text)
}

const SUMMARY_SYSTEM: &str = "You write release-note summaries. Reply with the summary text only.";

/// `claude -p` arguments for the summary. Safe mode skips CLAUDE.md, skills,
/// plugins, hooks and MCP servers, so user rules cannot add chatter.
pub fn summary_args(model: &str) -> Vec<&str> {
    vec![
        "-p",
        "--model",
        model,
        "--safe-mode",
        "--tools",
        "",
        "--no-session-persistence",
        "--system-prompt",
        SUMMARY_SYSTEM,
    ]
}

/// The cleaned summary from a `claude -p` reply, or why there is none.
pub fn summary_from_reply(raw: &str) -> Result<String> {
    if raw.trim().is_empty() {
        return Err(ReleaseError::EmptySummary);
    }
    clean_summary(raw).ok_or(ReleaseError::ChatterOnlySummary)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(subject: &str, ids: &[u32]) -> Commit {
        Commit {
            subject: subject.to_string(),
            ids: ids.to_vec(),
        }
    }

    fn ticket(id: u32, title: &str, labels: &[&str]) -> (u32, Ticket) {
        (
            id,
            Ticket {
                identifier: format!("GUZ-{id}"),
                title: title.to_string(),
                labels: labels.iter().map(|label| label.to_string()).collect(),
            },
        )
    }

    #[test]
    fn parse_log_collects_ids_from_subject_and_body_without_duplicates() {
        let raw = "a\x1ffix: one (GUZ-5)\x1fRefs GUZ-7 and GUZ-5\x1e\nb\x1fchore: bare\x1f\x1e\n";
        let commits = parse_log(raw);
        assert_eq!(
            commits,
            vec![
                commit("fix: one (GUZ-5)", &[5, 7]),
                commit("chore: bare", &[])
            ]
        );
    }

    #[test]
    fn parse_linear_treats_null_aliases_as_not_found() {
        let json = r#"{"data":{"t1":{"identifier":"GUZ-1","title":"One","labels":{"nodes":[{"name":"Bug"}]}},"t2":null}}"#;
        let tickets = parse_linear(json).unwrap();
        assert_eq!(tickets.len(), 1);
        assert_eq!(tickets[&1].title, "One");
        assert_eq!(tickets[&1].labels, vec!["Bug".to_string()]);
    }

    #[test]
    fn parse_linear_rejects_a_response_without_data() {
        assert!(parse_linear(r#"{"errors":[{"message":"x"}]}"#).is_err());
    }

    #[test]
    fn linear_query_aliases_each_id() {
        let query = linear_query(&[3, 9]);
        assert!(query.contains(r#"t3: issue(id: "GUZ-3")"#));
        assert!(query.contains(r#"t9: issue(id: "GUZ-9")"#));
    }

    #[test]
    fn label_decides_section_in_priority_order() {
        let commits = vec![commit("feat: a (GUZ-1)", &[1])];
        let tickets = HashMap::from([ticket(1, "Linear title", &["Bug", "Feature"])]);
        let notes = build_notes(&commits, &tickets);
        assert_eq!(notes.features, vec!["- Linear title (GUZ-1)".to_string()]);
        assert!(notes.fixes.is_empty());
    }

    #[test]
    fn unmapped_label_or_missing_ticket_falls_back_to_commit_type() {
        let commits = vec![
            commit("feat(x): add thing (GUZ-1)", &[1]),
            commit("fix!: repair (GUZ-2)", &[2]),
            commit("docs: words (GUZ-3)", &[3]),
        ];
        let tickets = HashMap::from([ticket(1, "Docs ticket", &["docs"])]);
        let notes = build_notes(&commits, &tickets);
        assert_eq!(notes.features, vec!["- Docs ticket (GUZ-1)".to_string()]);
        assert_eq!(notes.fixes, vec!["- fix!: repair (GUZ-2)".to_string()]);
        assert_eq!(
            notes.improvements,
            vec!["- docs: words (GUZ-3)".to_string()]
        );
    }

    #[test]
    fn one_entry_per_ticket_sorted_by_numeric_id() {
        let commits = vec![
            commit("fix: b (GUZ-10)", &[10]),
            commit("fix: a (GUZ-9)", &[9]),
            commit("fix: again (GUZ-10)", &[10]),
        ];
        let notes = build_notes(&commits, &HashMap::new());
        assert_eq!(
            notes.fixes,
            vec![
                "- fix: a (GUZ-9)".to_string(),
                "- fix: b (GUZ-10)".to_string()
            ]
        );
    }

    #[test]
    fn other_lists_only_feat_and_fix_and_counts_the_rest() {
        let commits = vec![
            commit("feat(ui)!: big", &[]),
            commit("fix: small", &[]),
            commit("chore: x", &[]),
            commit("not conventional", &[]),
        ];
        let notes = build_notes(&commits, &HashMap::new());
        assert_eq!(
            notes.other,
            vec![
                "- feat(ui)!: big".to_string(),
                "- fix: small".to_string(),
                "- plus 2 other commits".to_string()
            ]
        );
    }

    #[test]
    fn render_omits_empty_sections_and_includes_summary() {
        let notes = Notes {
            fixes: vec!["- F (GUZ-1)".to_string()],
            ..Notes::default()
        };
        assert_eq!(
            render_section("v0.3.0", "2026-11-01", Some("Summary."), &notes),
            "## v0.3.0 - 2026-11-01\n\nSummary.\n\n### Fixes\n\n- F (GUZ-1)\n"
        );
        assert_eq!(
            render_section("v0.3.0", "2026-11-01", None, &notes),
            "## v0.3.0 - 2026-11-01\n\n### Fixes\n\n- F (GUZ-1)\n"
        );
    }

    #[test]
    fn prepend_changelog_puts_newest_first_and_keeps_existing_sections() {
        let section = "## v2 - d\n\n### Fixes\n\n- x\n";
        let created = prepend_changelog(None, section);
        assert_eq!(created, "# Changelog\n\n## v2 - d\n\n### Fixes\n\n- x\n");
        let updated = prepend_changelog(Some(&created), "## v3 - e\n\n- y\n");
        assert_eq!(
            updated,
            "# Changelog\n\n## v3 - e\n\n- y\n\n## v2 - d\n\n### Fixes\n\n- x\n"
        );
    }

    #[test]
    fn summary_prompt_embeds_the_list_and_forbids_trailing_text() {
        assert_eq!(
            summary_prompt("- A (GUZ-1)"),
            "Write a two to three sentence summary of this release for its changelog. Plain prose, no heading, no bullet list, no preamble. Mention only what the list supports. Output only the summary text. Do not add questions, next steps, instructions or offers after it.\n\n- A (GUZ-1)"
        );
    }

    #[test]
    fn clean_summary_drops_a_trailing_next_line() {
        assert_eq!(
            clean_summary("Adds X. Fixes Y.\n\nnext: paste this above the v0.3.0 list… Go?\n"),
            Some("Adds X. Fixes Y.".to_string())
        );
    }

    #[test]
    fn clean_summary_drops_trailing_short_questions_and_offers() {
        assert_eq!(clean_summary("Adds X.\n\nGo?"), Some("Adds X.".to_string()));
        assert_eq!(
            clean_summary("Adds X.\n\nWant me to adjust the tone?\nLet me know if so."),
            Some("Adds X.".to_string())
        );
    }

    #[test]
    fn clean_summary_unwraps_code_fences() {
        assert_eq!(
            clean_summary("```text\nAdds X.\n```"),
            Some("Adds X.".to_string())
        );
    }

    #[test]
    fn clean_summary_drops_a_leading_preamble() {
        assert_eq!(
            clean_summary("Here's the summary:\n\nAdds X."),
            Some("Adds X.".to_string())
        );
    }

    #[test]
    fn clean_summary_keeps_a_long_first_line_that_ends_in_a_colon() {
        let text = "Summary: this release adds X, Y and Z:\n\n- X";
        assert_eq!(clean_summary(text), Some(text.to_string()));
    }

    #[test]
    fn clean_summary_keeps_questions_that_belong_to_the_summary() {
        let opening = "Why wait? This release adds X and Y.";
        assert_eq!(clean_summary(opening), Some(opening.to_string()));
        let closing = "This release adds X and Y. Who says changelogs must be boring?";
        assert_eq!(clean_summary(closing), Some(closing.to_string()));
        let wrapped = "This release adds X and Y, so is it\nready?";
        assert_eq!(clean_summary(wrapped), Some(wrapped.to_string()));
    }

    #[test]
    fn clean_summary_returns_none_when_nothing_is_left() {
        assert_eq!(clean_summary("  \n"), None);
        assert_eq!(clean_summary("```\n```"), None);
    }

    #[test]
    fn clean_summary_keeps_a_first_paragraph_that_ends_in_a_short_question() {
        let text = "Adds X and fixes Y. Ready to upgrade?";
        assert_eq!(clean_summary(text), Some(text.to_string()));
    }

    #[test]
    fn clean_summary_keeps_a_first_paragraph_with_a_chatter_phrase() {
        let text = "Fixes a bug where paste this command failed.";
        assert_eq!(clean_summary(text), Some(text.to_string()));
    }

    #[test]
    fn clean_summary_keeps_a_note_line_inside_the_first_paragraph() {
        let text = "Adds X.\nNote: requires Y.";
        assert_eq!(clean_summary(text), Some(text.to_string()));
    }

    #[test]
    fn clean_summary_keeps_clean_text() {
        assert_eq!(
            clean_summary("Adds X.\n\nFixes Y."),
            Some("Adds X.\n\nFixes Y.".to_string())
        );
    }

    #[test]
    fn summary_args_run_claude_without_tools_or_user_context() {
        assert_eq!(
            summary_args("m"),
            vec![
                "-p",
                "--model",
                "m",
                "--safe-mode",
                "--tools",
                "",
                "--no-session-persistence",
                "--system-prompt",
                "You write release-note summaries. Reply with the summary text only.",
            ]
        );
    }

    #[test]
    fn summary_from_reply_names_why_a_reply_gave_no_summary() {
        let reason = |raw: &str| summary_from_reply(raw).map_err(|error| error.to_string());
        assert_eq!(reason("Adds X.\n"), Ok("Adds X.".to_string()));
        assert_eq!(reason(" \n"), Err("claude returned no text".to_string()));
        assert_eq!(
            reason("Here's the summary:\n```\n```"),
            Err("the reply held only chatter".to_string())
        );
    }
}
