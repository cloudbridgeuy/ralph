use super::ids::{call_span, overlaps, parse_span, Span};
use super::parse::{Call, Source, Tool};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Opened {
    pub opened: usize,
    pub unopened: usize,
    pub unknown: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ZoomStats {
    pub depth: [usize; 5],
    pub size_count: usize,
    pub size_max: u64,
}

type TurnKey<'a> = (Option<&'a str>, &'a str);

fn turn_key(call: &Call) -> Option<TurnKey<'_>> {
    Some((call.run.as_deref(), call.turn.as_deref()?))
}

fn hit_spans(call: &Call) -> Vec<Span> {
    let scope = call.scope.as_deref();
    call.hit_ids
        .iter()
        .flatten()
        .filter_map(|id| parse_span(id, scope))
        .collect()
}

fn returned_hits(call: &Call) -> bool {
    matches!(call.tool, Tool::Recall | Tool::Search)
        && (call.hits.is_some_and(|hits| hits > 0)
            || call.hit_ids.as_ref().is_some_and(|ids| !ids.is_empty()))
}

fn has_hit_ids(call: &Call) -> bool {
    call.hit_ids.as_ref().is_some_and(|ids| !ids.is_empty())
}

pub fn count_opened(calls: &[Call]) -> Opened {
    let mut openers: BTreeMap<TurnKey<'_>, Vec<(usize, Span)>> = BTreeMap::new();
    for call in calls {
        if let (Some(key), Some(span)) = (turn_key(call), call_span(call)) {
            openers.entry(key).or_default().push((call.index, span));
        }
    }
    let mut found = Opened::default();
    for call in calls.iter().filter(|call| returned_hits(call)) {
        let tracked = has_hit_ids(call) && matches!(call.source, Source::Mcp | Source::Slash);
        let Some(key) = turn_key(call).filter(|_| tracked) else {
            found.unknown += 1;
            continue;
        };
        let hits = hit_spans(call);
        let later = openers.get(&key).into_iter().flatten();
        let reached = later
            .filter(|(index, _)| *index > call.index)
            .any(|(_, span)| hits.iter().any(|hit| overlaps(hit, span)));
        if reached {
            found.opened += 1;
        } else {
            found.unopened += 1;
        }
    }
    found
}

pub fn zoom_stats(calls: &[Call]) -> ZoomStats {
    let mut stats = ZoomStats::default();
    for call in calls {
        if call.tool == Tool::Zoom {
            if let Some(depth) = call.depth {
                let bucket = usize::try_from(depth).unwrap_or(usize::MAX).min(4);
                stats.depth[bucket] += 1;
            }
        }
        if let Some(size) = call.node_size {
            stats.size_count += 1;
            stats.size_max = stats.size_max.max(size);
        }
    }
    stats
}
