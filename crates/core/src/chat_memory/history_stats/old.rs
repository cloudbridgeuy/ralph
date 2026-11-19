use super::ids::{call_span, normalize_scope, view_low};
use super::parse::{Call, Tool, TurnRow, View};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OldReach {
    pub old: usize,
    pub decided: usize,
    pub undecided: usize,
}

struct TurnView {
    scope: Option<String>,
    start: Option<u64>,
}

fn own_start(row: &TurnRow, scope: Option<&str>) -> Option<u64> {
    match &row.view {
        View::Ids(ids) => ids.iter().filter_map(|id| view_low(id, scope)).min(),
        View::First(first) => view_low(first, scope),
    }
}

fn turn_views(turns: &[TurnRow]) -> HashMap<&str, TurnView> {
    let mut latest: HashMap<(Option<&str>, Option<String>), u64> = HashMap::new();
    let mut views = HashMap::new();
    for row in turns {
        let scope = row.scope.as_deref().and_then(normalize_scope);
        let key = (row.run.as_deref(), scope.clone());
        let own = own_start(row, scope.as_deref());
        let start = own.or_else(|| latest.get(&key).copied());
        if let Some(own) = own {
            latest.insert(key, own);
        }
        if let Some(turn) = row.turn.as_deref() {
            views.insert(turn, TurnView { scope, start });
        }
    }
    views
}

fn reach_of(call: &Call, view_scope: Option<&String>) -> Option<u64> {
    match call.tool {
        Tool::Zoom | Tool::Read => {
            let span = call_span(call)?;
            (span.scope.as_ref() == view_scope).then_some(span.lo)
        }
        Tool::Recall | Tool::Search => call.oldest_hit,
        Tool::Unknown => None,
    }
}

pub fn count_old(calls: &[Call], turns: &[TurnRow]) -> OldReach {
    let views = turn_views(turns);
    let mut found = OldReach::default();
    for call in calls {
        let view = call.turn.as_deref().and_then(|turn| views.get(turn));
        let scope = view.and_then(|view| view.scope.as_ref());
        let probe = match view {
            Some(_) => reach_of(call, scope),
            None => reach_of(
                call,
                call.scope.as_deref().and_then(normalize_scope).as_ref(),
            ),
        };
        let Some(reach) = probe else {
            continue;
        };
        match view.and_then(|view| view.start) {
            Some(start) => {
                found.decided += 1;
                if reach < start {
                    found.old += 1;
                }
            }
            None => found.undecided += 1,
        }
    }
    found
}
