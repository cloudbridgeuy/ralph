use crate::worker::{ticket_line, Ticket};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Remote<T> {
    Found(T),
    None,
    Failed,
    Pending,
}

impl<T> Remote<T> {
    pub fn cell(&self, show: impl Fn(&T) -> String) -> String {
        match self {
            Remote::Found(value) => show(value),
            Remote::None => "-".to_string(),
            Remote::Failed => "?".to_string(),
            Remote::Pending => "…".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrState {
    Open,
    Closed,
    Merged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Review {
    Approved,
    ChangesRequested,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pr {
    pub number: u64,
    pub state: PrState,
    pub draft: bool,
    pub review: Review,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketRemote {
    pub key: String,
    pub state: Remote<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LookupKey {
    Linear(String),
    Jira(String),
    Pr(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fetched {
    State(String),
    Pr(Option<Pr>),
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    at: u64,
    fetched: Fetched,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteCache {
    entries: HashMap<LookupKey, Entry>,
    in_flight: HashSet<LookupKey>,
}

fn is_fresh(entry: &Entry, now: u64, ttl: u64) -> bool {
    now.saturating_sub(entry.at) < ttl
}

impl RemoteCache {
    fn fresh(&self, key: &LookupKey, now: u64, ttl: u64) -> Option<&Fetched> {
        self.entries
            .get(key)
            .filter(|entry| is_fresh(entry, now, ttl))
            .map(|entry| &entry.fetched)
    }

    pub fn to_start(&self, wanted: &[LookupKey], now: u64, ttl: u64) -> Vec<LookupKey> {
        let mut seen = HashSet::new();
        wanted
            .iter()
            .filter(|key| self.fresh(key, now, ttl).is_none() && !self.in_flight.contains(*key))
            .filter(|key| seen.insert((*key).clone()))
            .cloned()
            .collect()
    }

    pub fn mark_in_flight(&mut self, keys: &[LookupKey]) {
        self.in_flight.extend(keys.iter().cloned());
    }

    pub fn store(&mut self, key: LookupKey, now: u64, fetched: Fetched) {
        self.in_flight.remove(&key);
        self.entries.insert(key, Entry { at: now, fetched });
    }

    pub fn ticket_state(&self, key: &LookupKey, now: u64, ttl: u64) -> Remote<String> {
        match self.fresh(key, now, ttl) {
            None => Remote::Pending,
            Some(Fetched::State(state)) => Remote::Found(state.clone()),
            Some(Fetched::Pr(_) | Fetched::Failed) => Remote::Failed,
        }
    }

    pub fn pr_state(&self, key: &LookupKey, now: u64, ttl: u64) -> Remote<Pr> {
        match self.fresh(key, now, ttl) {
            None => Remote::Pending,
            Some(Fetched::Pr(Some(pr))) => Remote::Found(pr.clone()),
            Some(Fetched::Pr(None)) => Remote::None,
            Some(Fetched::State(_) | Fetched::Failed) => Remote::Failed,
        }
    }
}

fn string_field(raw: &str, field: &str) -> Option<String> {
    let value: Value = serde_json::from_str(raw).ok()?;
    let text = value.get(field)?.as_str()?.trim();
    (!text.is_empty()).then(|| text.to_string())
}

pub fn parse_linear_state(raw: &str) -> Option<String> {
    string_field(raw, "state")
}

pub fn parse_jira_status(raw: &str) -> Option<String> {
    string_field(raw, "status")
}

#[derive(Deserialize)]
struct RawPr {
    number: u64,
    state: String,
    #[serde(default, rename = "isDraft")]
    is_draft: bool,
    #[serde(default)]
    url: String,
    #[serde(default, rename = "reviewDecision")]
    review_decision: Option<String>,
}

fn pr_of(raw: RawPr) -> Option<Pr> {
    let state = match raw.state.as_str() {
        "OPEN" => PrState::Open,
        "CLOSED" => PrState::Closed,
        "MERGED" => PrState::Merged,
        _ => return None,
    };
    let review = match raw.review_decision.as_deref() {
        Some("APPROVED") => Review::Approved,
        Some("CHANGES_REQUESTED") => Review::ChangesRequested,
        _ => Review::Other,
    };
    Some(Pr {
        number: raw.number,
        state,
        draft: raw.is_draft,
        review,
        url: raw.url,
    })
}

pub fn parse_pr_list(raw: &str) -> Option<Option<Pr>> {
    let list: Vec<Value> = serde_json::from_str(raw).ok()?;
    let Some(first) = list.into_iter().next() else {
        return Some(None);
    };
    let raw: RawPr = serde_json::from_value(first).ok()?;
    pr_of(raw).map(Some)
}

fn pr_label(pr: &Pr) -> String {
    let word = match (pr.state, pr.draft, pr.review) {
        (PrState::Merged, _, _) => "merged",
        (PrState::Closed, _, _) => "closed",
        (PrState::Open, true, _) => "draft",
        (PrState::Open, false, Review::Approved) => "approved",
        (PrState::Open, false, Review::ChangesRequested) => "changes",
        (PrState::Open, false, Review::Other) => "open",
    };
    format!("#{} {word}", pr.number)
}

pub fn pr_cell(pr: &Remote<Pr>) -> String {
    pr.cell(pr_label)
}

pub fn pr_detail(pr: &Remote<Pr>) -> String {
    pr.cell(|pr| {
        if pr.url.is_empty() {
            pr_label(pr)
        } else {
            format!("{} · {}", pr_label(pr), pr.url)
        }
    })
}

pub fn ticket_cell(worker_id: &str, ticket: &Option<TicketRemote>) -> String {
    match ticket {
        None => "-".to_string(),
        Some(ticket) => ticket.state.cell(|state| {
            if ticket.key == worker_id {
                state.clone()
            } else {
                format!("{} {state}", ticket.key)
            }
        }),
    }
}

pub fn ticket_detail(ticket: Option<&Ticket>, remote: &Option<TicketRemote>) -> String {
    match (ticket, remote) {
        (None, _) => "-".to_string(),
        (Some(ticket), None) => ticket_line(ticket),
        (Some(ticket), Some(remote)) => format!(
            "{} · {}",
            ticket_line(ticket),
            remote.state.cell(String::clone)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::Tracker;

    fn pr(state: PrState, draft: bool, review: Review) -> Pr {
        Pr {
            number: 12,
            state,
            draft,
            review,
            url: "https://github.com/o/r/pull/12".into(),
        }
    }

    fn linear(key: &str) -> LookupKey {
        LookupKey::Linear(key.into())
    }

    #[test]
    fn linear_state_and_jira_status_are_top_level_strings() {
        assert_eq!(
            parse_linear_state(r#"{"id":"x","state":"In Progress","n":1}"#),
            Some("In Progress".into())
        );
        assert_eq!(
            parse_jira_status(r#"{"status":" Ready to Implement "}"#),
            Some("Ready to Implement".into())
        );
    }

    #[test]
    fn missing_empty_or_wrong_typed_fields_do_not_parse() {
        for raw in [
            "",
            "garbage",
            "[]",
            "{}",
            r#"{"state":""}"#,
            r#"{"state":3}"#,
            r#"{"status":"Done"}"#,
            r#"{"state":{"name":"Done"}}"#,
        ] {
            assert_eq!(parse_linear_state(raw), None, "{raw}");
        }
        assert_eq!(parse_jira_status(r#"{"state":"Done"}"#), None);
        assert_eq!(parse_jira_status(r#"{"status":null}"#), None);
    }

    #[test]
    fn an_empty_pr_list_is_no_pr_and_the_first_item_wins() {
        assert_eq!(parse_pr_list("[]"), Some(None));
        let raw = r#"[
            {"number":12,"state":"OPEN","isDraft":true,"url":"u12","reviewDecision":"APPROVED"},
            {"number":3,"state":"MERGED","isDraft":false,"url":"u3","reviewDecision":""}
        ]"#;
        assert_eq!(
            parse_pr_list(raw),
            Some(Some(Pr {
                number: 12,
                state: PrState::Open,
                draft: true,
                review: Review::Approved,
                url: "u12".into(),
            }))
        );
    }

    #[test]
    fn review_decisions_map_and_null_or_missing_is_other() {
        let review = |item: &str| parse_pr_list(&format!("[{item}]")).unwrap().unwrap().review;
        let base = r#""number":1,"state":"OPEN""#;
        assert_eq!(
            review(&format!(
                r#"{{{base},"reviewDecision":"CHANGES_REQUESTED"}}"#
            )),
            Review::ChangesRequested
        );
        assert_eq!(
            review(&format!(r#"{{{base},"reviewDecision":"REVIEW_REQUIRED"}}"#)),
            Review::Other
        );
        assert_eq!(
            review(&format!(r#"{{{base},"reviewDecision":null}}"#)),
            Review::Other
        );
        assert_eq!(review(&format!("{{{base}}}")), Review::Other);
    }

    #[test]
    fn every_pr_state_parses_and_unknown_ones_do_not() {
        let state = |word: &str| {
            parse_pr_list(&format!(r#"[{{"number":1,"state":"{word}"}}]"#))
                .map(|p| p.unwrap().state)
        };
        assert_eq!(state("OPEN"), Some(PrState::Open));
        assert_eq!(state("CLOSED"), Some(PrState::Closed));
        assert_eq!(state("MERGED"), Some(PrState::Merged));
        assert_eq!(state("open"), None);
    }

    #[test]
    fn unparsable_pr_output_is_none_not_no_pr() {
        for raw in [
            "",
            "garbage",
            "{}",
            "[3]",
            r#"[{"state":"OPEN"}]"#,
            r#"[{"number":1}]"#,
            r#"[{"number":"1","state":"OPEN"}]"#,
        ] {
            assert_eq!(parse_pr_list(raw), None, "{raw}");
        }
    }

    #[test]
    fn pr_cells_name_the_state_and_review() {
        let cell = |state, draft, review| pr_cell(&Remote::Found(pr(state, draft, review)));
        assert_eq!(cell(PrState::Open, true, Review::Other), "#12 draft");
        assert_eq!(cell(PrState::Open, true, Review::Approved), "#12 draft");
        assert_eq!(cell(PrState::Open, false, Review::Other), "#12 open");
        assert_eq!(cell(PrState::Open, false, Review::Approved), "#12 approved");
        assert_eq!(
            cell(PrState::Open, false, Review::ChangesRequested),
            "#12 changes"
        );
        assert_eq!(cell(PrState::Merged, false, Review::Approved), "#12 merged");
        assert_eq!(cell(PrState::Closed, true, Review::Other), "#12 closed");
    }

    #[test]
    fn placeholder_cells_are_dash_question_and_ellipsis() {
        assert_eq!(pr_cell(&Remote::None), "-");
        assert_eq!(pr_cell(&Remote::Failed), "?");
        assert_eq!(pr_cell(&Remote::Pending), "…");
    }

    #[test]
    fn pr_detail_appends_the_url_when_known() {
        let open = pr(PrState::Open, false, Review::Other);
        assert_eq!(
            pr_detail(&Remote::Found(open.clone())),
            "#12 open · https://github.com/o/r/pull/12"
        );
        let bare = Pr {
            url: String::new(),
            ..open
        };
        assert_eq!(pr_detail(&Remote::Found(bare)), "#12 open");
        assert_eq!(pr_detail(&Remote::Pending), "…");
        assert_eq!(pr_detail(&Remote::None), "-");
    }

    fn remote(key: &str, state: Remote<String>) -> Option<TicketRemote> {
        Some(TicketRemote {
            key: key.into(),
            state,
        })
    }

    #[test]
    fn the_ticket_cell_prefixes_the_key_only_when_it_differs_from_the_id() {
        let found = || Remote::Found("In Progress".to_string());
        assert_eq!(
            ticket_cell("GUZ-1", &remote("GUZ-1", found())),
            "In Progress"
        );
        assert_eq!(
            ticket_cell("fix-x", &remote("GUZ-1", found())),
            "GUZ-1 In Progress"
        );
        assert_eq!(ticket_cell("GUZ-1", &None), "-");
        assert_eq!(ticket_cell("fix-x", &remote("GUZ-1", Remote::Pending)), "…");
        assert_eq!(ticket_cell("fix-x", &remote("GUZ-1", Remote::Failed)), "?");
        assert_eq!(ticket_cell("fix-x", &remote("GUZ-1", Remote::None)), "-");
    }

    #[test]
    fn the_ticket_detail_joins_the_ticket_line_and_the_state() {
        let ticket = Ticket {
            key: "GUZ-846".into(),
            tracker: Some(Tracker::Linear),
        };
        let found = remote("GUZ-846", Remote::Found("In Progress".into()));
        assert_eq!(
            ticket_detail(Some(&ticket), &found),
            "GUZ-846 (linear) · In Progress"
        );
        assert_eq!(
            ticket_detail(Some(&ticket), &remote("GUZ-846", Remote::Pending)),
            "GUZ-846 (linear) · …"
        );
        assert_eq!(
            ticket_detail(Some(&ticket), &remote("GUZ-846", Remote::Failed)),
            "GUZ-846 (linear) · ?"
        );
        assert_eq!(ticket_detail(Some(&ticket), &None), "GUZ-846 (linear)");
        assert_eq!(ticket_detail(None, &None), "-");
    }

    #[test]
    fn missing_keys_start_once_and_in_flight_keys_do_not() {
        let mut cache = RemoteCache::default();
        let wanted = [linear("A-1"), linear("A-1"), LookupKey::Pr("w".into())];
        let start = cache.to_start(&wanted, 100, 60);
        assert_eq!(start, [linear("A-1"), LookupKey::Pr("w".into())]);
        cache.mark_in_flight(&start);
        assert!(cache.to_start(&wanted, 101, 60).is_empty());
        assert_eq!(cache.to_start(&[linear("B-2")], 101, 60), [linear("B-2")]);
    }

    #[test]
    fn a_stored_result_clears_in_flight_and_is_fresh_until_the_ttl() {
        let mut cache = RemoteCache::default();
        cache.mark_in_flight(&[linear("A-1")]);
        cache.store(linear("A-1"), 100, Fetched::State("Done".into()));
        assert!(cache.to_start(&[linear("A-1")], 159, 60).is_empty());
        assert_eq!(
            cache.ticket_state(&linear("A-1"), 159, 60),
            Remote::Found("Done".into())
        );
        assert_eq!(cache.to_start(&[linear("A-1")], 160, 60), [linear("A-1")]);
        assert_eq!(cache.ticket_state(&linear("A-1"), 160, 60), Remote::Pending);
    }

    #[test]
    fn failures_are_cached_like_successes() {
        let mut cache = RemoteCache::default();
        cache.store(linear("A-1"), 100, Fetched::Failed);
        assert_eq!(cache.ticket_state(&linear("A-1"), 130, 60), Remote::Failed);
        assert!(cache.to_start(&[linear("A-1")], 130, 60).is_empty());
        let key = LookupKey::Pr("w".into());
        cache.store(key.clone(), 100, Fetched::Failed);
        assert_eq!(cache.pr_state(&key, 130, 60), Remote::Failed);
    }

    #[test]
    fn pr_results_read_back_as_found_or_none() {
        let mut cache = RemoteCache::default();
        let key = LookupKey::Pr("w".into());
        assert_eq!(cache.pr_state(&key, 0, 60), Remote::Pending);
        cache.store(key.clone(), 10, Fetched::Pr(None));
        assert_eq!(cache.pr_state(&key, 20, 60), Remote::None);
        let open = pr(PrState::Open, false, Review::Other);
        cache.store(key.clone(), 10, Fetched::Pr(Some(open.clone())));
        assert_eq!(cache.pr_state(&key, 20, 60), Remote::Found(open));
    }

    #[test]
    fn a_result_of_the_wrong_kind_reads_as_failed() {
        let mut cache = RemoteCache::default();
        cache.store(linear("A-1"), 10, Fetched::Pr(None));
        assert_eq!(cache.ticket_state(&linear("A-1"), 20, 60), Remote::Failed);
        let key = LookupKey::Pr("w".into());
        cache.store(key.clone(), 10, Fetched::State("x".into()));
        assert_eq!(cache.pr_state(&key, 20, 60), Remote::Failed);
    }

    #[test]
    fn a_zero_ttl_is_never_fresh_and_a_future_stamp_is_fresh() {
        let mut cache = RemoteCache::default();
        cache.store(linear("A-1"), 100, Fetched::State("x".into()));
        assert_eq!(cache.ticket_state(&linear("A-1"), 100, 0), Remote::Pending);
        assert_eq!(
            cache.ticket_state(&linear("A-1"), 50, 10),
            Remote::Found("x".into())
        );
    }

    #[test]
    fn jira_and_linear_keys_with_one_name_are_distinct() {
        let mut cache = RemoteCache::default();
        cache.store(linear("A-1"), 10, Fetched::State("x".into()));
        assert_eq!(
            cache.to_start(&[LookupKey::Jira("A-1".into())], 20, 60),
            [LookupKey::Jira("A-1".into())]
        );
    }
}
