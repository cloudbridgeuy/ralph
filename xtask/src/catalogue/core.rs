use chrono::DateTime;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const MAX_BYTES: usize = 20 * 1024;
pub const MAX_TITLES: usize = 40;
pub const MAX_TITLE_CHARS: usize = 200;
pub const MAX_DAYS: usize = 60;
pub const VERY_LONG_CHARS: usize = 300;
pub const FORBIDDEN: [&str; 4] = ["sk-", "/Users/", "@", "token"];

#[derive(Debug, Clone, PartialEq)]
pub struct DayCost {
    pub day: String,
    pub turns: u64,
    pub cost_usd: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Catalogue {
    pub titles: Vec<String>,
    pub longest_title: Option<String>,
    pub very_long_title_derived: Option<String>,
    pub log_entries: u64,
    pub user_entries: u64,
    pub assistant_entries: u64,
    pub turns: u64,
    pub naps: u64,
    pub workers: u64,
    pub turn_total_ms: u64,
    pub turn_median_ms: u64,
    pub turn_max_ms: u64,
    pub cost_series: Vec<DayCost>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Turn,
    Nap,
}

#[derive(Debug, Clone, PartialEq)]
struct Usage {
    day: String,
    kind: Kind,
    cost_usd: f64,
    duration_ms: u64,
}

fn json_lines(text: &str) -> impl Iterator<Item = Value> + '_ {
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
}

fn is_day(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                *b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
}

fn usage_row(value: &Value) -> Option<Usage> {
    let kind = match value.get("kind")?.as_str()? {
        "turn" => Kind::Turn,
        "nap" => Kind::Nap,
        _ => return None,
    };
    let day = value.get("at")?.as_str()?.get(..10)?;
    if !is_day(day) {
        return None;
    }
    let cost_usd = value
        .get("cost_usd")?
        .as_f64()
        .filter(|c| c.is_finite() && *c >= 0.0)?;
    let duration_ms = value.get("duration_ms")?.as_u64()?;
    Some(Usage {
        day: day.to_string(),
        kind,
        cost_usd,
        duration_ms,
    })
}

fn is_safe_title(line: &str) -> bool {
    let lower = line.to_lowercase();
    !line.is_empty()
        && line.chars().count() <= MAX_TITLE_CHARS
        && !line.chars().any(char::is_control)
        && !line.contains(['/', '\\'])
        && !FORBIDDEN
            .iter()
            .any(|word| lower.contains(&word.to_lowercase()))
}

fn is_timestamp(line: &str) -> bool {
    DateTime::parse_from_rfc3339(line).is_ok()
}

fn status_titles(statuses: &[&str]) -> Vec<String> {
    let mut titles: Vec<String> = Vec::new();
    for line in statuses.iter().flat_map(|text| text.lines()) {
        let line = line.trim();
        if is_timestamp(line) || !is_safe_title(line) || titles.iter().any(|t| t == line) {
            continue;
        }
        titles.push(line.to_string());
    }
    titles.truncate(MAX_TITLES);
    titles
}

fn repeat_to_very_long(title: &str) -> String {
    let mut text = title.to_string();
    while text.chars().count() < VERY_LONG_CHARS {
        text.push(' ');
        text.push_str(title);
    }
    text
}

fn day_costs(rows: &[Usage]) -> Vec<DayCost> {
    let mut days: BTreeMap<&str, (u64, f64)> = BTreeMap::new();
    for row in rows {
        let entry = days.entry(row.day.as_str()).or_default();
        entry.0 += u64::from(row.kind == Kind::Turn);
        entry.1 += row.cost_usd;
    }
    let mut series: Vec<DayCost> = days
        .into_iter()
        .map(|(day, (turns, cost))| DayCost {
            day: day.to_string(),
            turns,
            cost_usd: cost.round() as u64,
        })
        .collect();
    let skip = series.len().saturating_sub(MAX_DAYS);
    series.drain(..skip);
    series
}

fn role_count(entries: &[Value], role: &str) -> u64 {
    entries
        .iter()
        .filter(|entry| entry.get("role").and_then(Value::as_str) == Some(role))
        .count() as u64
}

pub fn derive(log: &str, usage: &str, statuses: &[&str]) -> Catalogue {
    let entries: Vec<Value> = json_lines(log).collect();
    let rows: Vec<Usage> = json_lines(usage).filter_map(|v| usage_row(&v)).collect();
    let mut durations: Vec<u64> = rows
        .iter()
        .filter(|row| row.kind == Kind::Turn)
        .map(|row| row.duration_ms)
        .collect();
    durations.sort_unstable();
    let titles = status_titles(statuses);
    let longest_title = titles
        .iter()
        .fold(None::<&String>, |best, title| match best {
            Some(current) if current.chars().count() >= title.chars().count() => Some(current),
            _ => Some(title),
        })
        .cloned();
    Catalogue {
        titles,
        very_long_title_derived: longest_title.as_deref().map(repeat_to_very_long),
        longest_title,
        log_entries: entries.len() as u64,
        user_entries: role_count(&entries, "user"),
        assistant_entries: role_count(&entries, "assistant"),
        turns: durations.len() as u64,
        naps: rows.iter().filter(|row| row.kind == Kind::Nap).count() as u64,
        workers: statuses.len() as u64,
        turn_total_ms: durations.iter().sum(),
        turn_median_ms: durations.get(durations.len() / 2).copied().unwrap_or(0),
        turn_max_ms: durations.last().copied().unwrap_or(0),
        cost_series: day_costs(&rows),
    }
}

pub fn leaks(text: &str) -> Vec<&'static str> {
    let lower = text.to_lowercase();
    FORBIDDEN
        .into_iter()
        .filter(|word| lower.contains(&word.to_lowercase()))
        .collect()
}

pub fn render(catalogue: &Catalogue) -> Result<String, String> {
    let value = json!({
        "schema": 1,
        "titles": catalogue.titles,
        "longest_title": catalogue.longest_title,
        "very_long_title_derived": catalogue.very_long_title_derived,
        "counts": {
            "log_entries": catalogue.log_entries,
            "user_entries": catalogue.user_entries,
            "assistant_entries": catalogue.assistant_entries,
            "turns": catalogue.turns,
            "naps": catalogue.naps,
            "workers": catalogue.workers,
        },
        "durations_ms": {
            "turn_total": catalogue.turn_total_ms,
            "turn_median": catalogue.turn_median_ms,
            "turn_max": catalogue.turn_max_ms,
        },
        "cost_series_usd": catalogue
            .cost_series
            .iter()
            .map(|d| json!({"day": d.day, "turns": d.turns, "cost": d.cost_usd}))
            .collect::<Vec<_>>(),
    });
    let text = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())? + "\n";
    let found = leaks(&text);
    if !found.is_empty() {
        return Err(format!("the fixture holds forbidden text: {found:?}"));
    }
    if text.len() > MAX_BYTES {
        return Err(format!(
            "the fixture is {} bytes; the cap is {MAX_BYTES}",
            text.len()
        ));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = concat!(
        "{\"at\":\"2026-10-01T10:00:00+00:00\",\"role\":\"user\",\"text\":\"my key is sk-abc /Users/me/x a@b.c\"}\n",
        "{\"at\":\"2026-10-01T10:00:01+00:00\",\"role\":\"assistant\",\"text\":\"secret body\"}\n",
        "not json\n",
        "{\"at\":\"2026-10-01T10:00:02+00:00\",\"role\":\"assistant\",\"text\":\"more\"}\n",
    );

    const USAGE: &str = concat!(
        "{\"at\":\"2026-10-01T10:00:00+00:00\",\"kind\":\"turn\",\"cost_usd\":0.1,\"duration_ms\":1000,\"prompt\":\"leak\"}\n",
        "{\"at\":\"2026-10-01T11:00:00+00:00\",\"kind\":\"turn\",\"cost_usd\":0.25,\"duration_ms\":3000}\n",
        "{\"at\":\"2026-10-01T12:00:00+00:00\",\"kind\":\"nap\",\"cost_usd\":0.05,\"duration_ms\":500}\n",
        "{\"at\":\"2026-10-02T12:00:00+00:00\",\"kind\":\"turn\",\"cost_usd\":0.5,\"duration_ms\":2000}\n",
        "{\"at\":\"2026-10-02T12:00:00+00:00\",\"kind\":\"other\",\"cost_usd\":9,\"duration_ms\":1}\n",
        "{\"at\":\"bad\",\"kind\":\"turn\",\"cost_usd\":9,\"duration_ms\":1}\n",
        "{\"at\":\"2026-10-03T12:00:00+00:00\",\"kind\":\"turn\",\"cost_usd\":-1,\"duration_ms\":1}\n",
    );

    const STATUS: &str = concat!(
        "2026-11-22T14:47:10.818729+00:00\n",
        "Reading the plan\n",
        "2026-11-22T14:48:10.818729+00:00\n",
        "Editing /Users/me/secret.rs\n",
        "2026-11-22T14:49:10.818729+00:00\n",
        "Sending the token to a@b.c with sk-123\n",
        "2026-11-22T14:50:10.818729+00:00\n",
        "Reading the plan\n",
        "2026-11-22T14:51:10.818729+00:00\n",
        "A longer title that is still safe to show\n",
    );

    #[test]
    fn counts_only_known_roles_and_valid_lines() {
        let c = derive(LOG, USAGE, &[STATUS]);
        assert_eq!(
            (c.log_entries, c.user_entries, c.assistant_entries),
            (3, 1, 2)
        );
    }

    #[test]
    fn counts_turns_naps_and_durations_from_valid_rows() {
        let c = derive(LOG, USAGE, &[STATUS]);
        assert_eq!((c.turns, c.naps), (3, 1));
        assert_eq!(c.turn_total_ms, 6000);
        assert_eq!(c.turn_median_ms, 2000);
        assert_eq!(c.turn_max_ms, 3000);
    }

    #[test]
    fn cost_series_sums_each_day() {
        let c = derive(LOG, USAGE, &[STATUS]);
        assert_eq!(
            c.cost_series,
            vec![
                DayCost {
                    day: "2026-10-01".into(),
                    turns: 2,
                    cost_usd: 0
                },
                DayCost {
                    day: "2026-10-02".into(),
                    turns: 1,
                    cost_usd: 1
                },
            ]
        );
    }

    #[test]
    fn cost_series_keeps_only_the_latest_days() {
        let usage: String = (0..90)
            .map(|d| {
                format!(
                    "{{\"at\":\"2026-{:02}-{:02}T00:00:00+00:00\",\"kind\":\"turn\",\"cost_usd\":1,\"duration_ms\":1}}\n",
                    1 + d / 28,
                    1 + d % 28
                )
            })
            .collect();
        let c = derive("", &usage, &[]);
        assert_eq!(c.cost_series.len(), MAX_DAYS);
        assert_eq!(
            c.cost_series.last().map(|d| d.day.as_str()),
            Some("2026-04-06")
        );
    }

    #[test]
    fn titles_drop_paths_secrets_timestamps_and_repeats() {
        let c = derive("", "", &[STATUS]);
        assert_eq!(
            c.titles,
            vec![
                "Reading the plan",
                "A longer title that is still safe to show"
            ]
        );
        assert_eq!(
            c.longest_title.as_deref(),
            Some("A longer title that is still safe to show")
        );
    }

    #[test]
    fn a_title_over_the_cap_is_dropped_not_cut() {
        let long = "x".repeat(MAX_TITLE_CHARS + 1);
        assert!(derive("", "", &[&long]).titles.is_empty());
        let fits = "x".repeat(MAX_TITLE_CHARS);
        assert_eq!(derive("", "", &[&fits]).titles, vec![fits]);
    }

    #[test]
    fn empty_input_gives_empty_catalogue() {
        let c = derive("", "", &[]);
        assert_eq!(c.longest_title, None);
        assert_eq!((c.turns, c.turn_median_ms, c.turn_max_ms), (0, 0, 0));
        assert!(c.cost_series.is_empty());
    }

    #[test]
    fn rendered_fixture_has_no_message_body_prompt_path_or_secret() {
        let text = render(&derive(LOG, USAGE, &[STATUS])).unwrap();
        for needle in ["secret body", "leak", "sk-", "/Users/", "@", "token"] {
            assert!(!text.to_lowercase().contains(needle), "{needle}");
        }
        assert!(text.contains("\"cost_series_usd\""));
    }

    #[test]
    fn leaks_flags_each_forbidden_word() {
        assert_eq!(leaks("clean"), Vec::<&str>::new());
        assert_eq!(leaks("sk-1 /Users/a b@c Token"), FORBIDDEN.to_vec());
    }

    #[test]
    fn render_refuses_forbidden_text() {
        let mut c = derive("", "", &[]);
        c.titles = vec!["has an @ sign".into()];
        assert!(render(&c).is_err());
    }

    #[test]
    fn worst_case_fixture_fits_the_size_cap() {
        let mut c = derive("", "", &[]);
        c.titles = (0..MAX_TITLES)
            .map(|i| format!("{i:02}{}", "w".repeat(MAX_TITLE_CHARS - 2)))
            .collect();
        c.cost_series = (0..MAX_DAYS)
            .map(|i| DayCost {
                day: format!("2026-01-{:02}", 1 + i % 28),
                turns: 123_456,
                cost_usd: 12_346,
            })
            .collect();
        assert!(render(&c).unwrap().len() <= MAX_BYTES);
    }

    #[test]
    fn render_refuses_an_oversized_fixture() {
        let mut c = derive("", "", &[]);
        c.titles = vec!["y".repeat(MAX_TITLE_CHARS); 200];
        assert!(render(&c).unwrap_err().contains("cap"));
    }

    #[test]
    fn very_long_title_repeats_the_longest_real_title() {
        let c = derive("", "", &[STATUS]);
        let long = c.very_long_title_derived.unwrap();
        assert!(long.chars().count() >= VERY_LONG_CHARS);
        assert!(long.chars().count() < VERY_LONG_CHARS + 100);
        let title = "A longer title that is still safe to show";
        assert!(long.starts_with(title));
        assert_eq!(
            long.split(title)
                .filter(|part| !part.trim().is_empty())
                .count(),
            0
        );
    }

    #[test]
    fn no_title_gives_no_very_long_title() {
        assert_eq!(derive("", "", &[]).very_long_title_derived, None);
    }

    #[test]
    fn costs_round_to_whole_dollars() {
        let usage = "{\"at\":\"2026-10-01T00:00:00+00:00\",\"kind\":\"turn\",\"cost_usd\":2.6,\"duration_ms\":1}\n";
        assert_eq!(derive("", usage, &[]).cost_series[0].cost_usd, 3);
    }

    #[test]
    fn committed_fixture_is_clean_and_under_the_cap() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../crates/ralph_api/fixtures/catalogue.json");
        let text = std::fs::read_to_string(path).unwrap();
        assert_eq!(leaks(&text), Vec::<&str>::new());
        assert!(text.len() <= MAX_BYTES, "{} bytes", text.len());
        let value: Value = serde_json::from_str(&text).unwrap();
        assert!(value["very_long_title_derived"].as_str().is_some());
        assert!(value["cost_series_usd"][0]["cost"].is_u64());
    }
}
