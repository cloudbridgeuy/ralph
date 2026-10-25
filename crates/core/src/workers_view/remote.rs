use crate::worker::{branch_name, ticket_line, Ticket, Tracker};
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LookupCommand {
    pub program: &'static str,
    pub args: Vec<String>,
}

pub fn ticket_lookup(ticket: &Ticket) -> Option<LookupKey> {
    match ticket.tracker? {
        Tracker::Linear => Some(LookupKey::Linear(ticket.key.clone())),
        Tracker::Jira => Some(LookupKey::Jira(ticket.key.clone())),
    }
}

pub fn wanted_keys(id: &str, ticket: Option<&Ticket>, pr_enabled: bool) -> Vec<LookupKey> {
    ticket
        .and_then(ticket_lookup)
        .into_iter()
        .chain(pr_enabled.then(|| LookupKey::Pr(id.to_string())))
        .collect()
}

pub fn lookup_command(key: &LookupKey) -> LookupCommand {
    let (program, args): (&'static str, Vec<String>) = match key {
        LookupKey::Linear(ticket) => (
            "mcptools",
            [
                "linear",
                "issue",
                "get",
                ticket,
                "--history-limit",
                "0",
                "--json",
            ]
            .map(String::from)
            .to_vec(),
        ),
        LookupKey::Jira(ticket) => (
            "mcptools",
            ["atlassian", "jira", "get", ticket, "--json"]
                .map(String::from)
                .to_vec(),
        ),
        LookupKey::Pr(id) => (
            "gh",
            [
                "pr",
                "list",
                "--head",
                &branch_name(id),
                "--state",
                "all",
                "--json",
                "number,state,isDraft,url,reviewDecision",
            ]
            .map(String::from)
            .to_vec(),
        ),
    };
    LookupCommand { program, args }
}

pub fn fetched_of(key: &LookupKey, output: Option<&str>) -> Fetched {
    let parsed = output.and_then(|raw| match key {
        LookupKey::Linear(_) => parse_linear_state(raw).map(Fetched::State),
        LookupKey::Jira(_) => parse_jira_status(raw).map(Fetched::State),
        LookupKey::Pr(_) => parse_pr_list(raw).map(Fetched::Pr),
    });
    parsed.unwrap_or(Fetched::Failed)
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

    fn ticket(key: &str, tracker: Option<Tracker>) -> Ticket {
        Ticket {
            key: key.into(),
            tracker,
        }
    }

    #[test]
    fn a_ticket_looks_up_only_with_a_tracker() {
        assert_eq!(
            ticket_lookup(&ticket("GUZ-1", Some(Tracker::Linear))),
            Some(linear("GUZ-1"))
        );
        assert_eq!(
            ticket_lookup(&ticket("VP20-1", Some(Tracker::Jira))),
            Some(LookupKey::Jira("VP20-1".into()))
        );
        assert_eq!(ticket_lookup(&ticket("FOO-1", None)), None);
    }

    #[test]
    fn wanted_keys_follow_the_tracker_and_the_pr_switch() {
        let linear_ticket = ticket("GUZ-1", Some(Tracker::Linear));
        assert_eq!(
            wanted_keys("GUZ-1", Some(&linear_ticket), true),
            [linear("GUZ-1"), LookupKey::Pr("GUZ-1".into())]
        );
        assert_eq!(
            wanted_keys("GUZ-1", Some(&linear_ticket), false),
            [linear("GUZ-1")]
        );
        let jira_ticket = ticket("VP20-5", Some(Tracker::Jira));
        assert_eq!(
            wanted_keys("w", Some(&jira_ticket), false),
            [LookupKey::Jira("VP20-5".into())]
        );
        let bare = ticket("FOO-1", None);
        assert_eq!(
            wanted_keys("w", Some(&bare), true),
            [LookupKey::Pr("w".into())]
        );
        assert_eq!(wanted_keys("w", None, true), [LookupKey::Pr("w".into())]);
        assert!(wanted_keys("w", None, false).is_empty());
    }

    #[test]
    fn lookup_commands_match_the_cli_contracts() {
        let command = lookup_command(&linear("GUZ-846"));
        assert_eq!(command.program, "mcptools");
        assert_eq!(
            command.args,
            [
                "linear",
                "issue",
                "get",
                "GUZ-846",
                "--history-limit",
                "0",
                "--json"
            ]
        );
        let command = lookup_command(&LookupKey::Jira("VP20-5".into()));
        assert_eq!(command.program, "mcptools");
        assert_eq!(
            command.args,
            ["atlassian", "jira", "get", "VP20-5", "--json"]
        );
        let command = lookup_command(&LookupKey::Pr("GUZ-846".into()));
        assert_eq!(command.program, "gh");
        assert_eq!(
            command.args,
            [
                "pr",
                "list",
                "--head",
                "worker/GUZ-846",
                "--state",
                "all",
                "--json",
                "number,state,isDraft,url,reviewDecision"
            ]
        );
    }

    #[test]
    fn fetched_of_parses_by_key_kind_and_fails_on_no_output_or_bad_output() {
        assert_eq!(
            fetched_of(&linear("A-1"), Some(r#"{"state":"Done"}"#)),
            Fetched::State("Done".into())
        );
        assert_eq!(
            fetched_of(
                &LookupKey::Jira("A-1".into()),
                Some(r#"{"status":"To Do"}"#)
            ),
            Fetched::State("To Do".into())
        );
        assert_eq!(
            fetched_of(&LookupKey::Jira("A-1".into()), Some(r#"{"state":"x"}"#)),
            Fetched::Failed
        );
        assert_eq!(
            fetched_of(&LookupKey::Pr("w".into()), Some("[]")),
            Fetched::Pr(None)
        );
        assert_eq!(
            fetched_of(&LookupKey::Pr("w".into()), Some("nope")),
            Fetched::Failed
        );
        assert_eq!(fetched_of(&linear("A-1"), None), Fetched::Failed);
    }
}
