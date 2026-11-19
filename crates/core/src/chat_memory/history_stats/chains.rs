use super::config::StatsConfig;
use super::parse::{Call, Outcome, Source, Tool};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Chains {
    pub chains: usize,
    pub calls_in_chains: usize,
}

fn is_search_call(call: &Call) -> bool {
    matches!(call.tool, Tool::Recall | Tool::Search)
        && matches!(call.source, Source::Mcp | Source::Slash)
        && call.turn.is_some()
}

fn weak_hits(call: &Call, few_hits: u64) -> bool {
    let hits = match call.outcome {
        Outcome::Empty => Some(call.hits.unwrap_or(0)),
        Outcome::Ok => call.hits,
        Outcome::Error | Outcome::Unknown => None,
    };
    hits.is_some_and(|hits| hits <= few_hits)
}

fn close(run: usize, cfg: &StatsConfig, found: &mut Chains) {
    if run >= cfg.chain_min {
        found.chains += 1;
        found.calls_in_chains += run;
    }
}

fn scan_turn(calls: &[&Call], cfg: &StatsConfig, found: &mut Chains) {
    let gap = i64::try_from(cfg.gap_secs).unwrap_or(i64::MAX);
    let mut run = 0;
    let mut previous: Option<i64> = None;
    for call in calls {
        let gapped = previous.is_some_and(|at| call.at.saturating_sub(at) > gap);
        previous = Some(call.at);
        if gapped {
            close(run, cfg, found);
            run = 0;
        }
        if weak_hits(call, cfg.few_hits) {
            run += 1;
        } else {
            close(run, cfg, found);
            run = 0;
        }
    }
    close(run, cfg, found);
}

pub fn build_chains(calls: &[Call], cfg: &StatsConfig) -> Chains {
    let mut turns: BTreeMap<(Option<&str>, &str), Vec<&Call>> = BTreeMap::new();
    for call in calls.iter().filter(|call| is_search_call(call)) {
        if let Some(turn) = call.turn.as_deref() {
            turns
                .entry((call.run.as_deref(), turn))
                .or_default()
                .push(call);
        }
    }
    let mut found = Chains::default();
    for group in turns.values() {
        scan_turn(group, cfg, &mut found);
    }
    found
}
