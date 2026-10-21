use std::collections::BTreeSet;

use serde::Serialize;
use serde_json::Value;

use super::provider::{CallKind, PromptSegment, Provider};
use super::read::ReadError;
use super::record::{CallRecord, RequestBody};
use super::usage::{CacheTier, Usage, UsageOutcome};

const EXCERPT_BYTES: usize = 40;
pub const CACHE_AGREE_RATIO: f64 = 0.9;

const INVISIBLE_CAUSES: [&str; 3] = [
    "cache eviction",
    "different account or workspace (wire stores no credential fingerprint)",
    "provider-side change",
];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FactDiff {
    pub name: String,
    pub a: Option<Value>,
    pub b: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HeaderDiff {
    pub name: String,
    pub a: Option<String>,
    pub b: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Divergence {
    pub index: usize,
    pub label_a: String,
    pub label_b: String,
    pub offset: usize,
    pub a_excerpt: String,
    pub b_excerpt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Verdict {
    NoSegments,
    NoBase,
    NoSharedBreakpoint,
    UsageMissing,
    PrefixChanged {
        label: String,
    },
    PrefixShared {
        relation: Relation,
    },
    ModelChanged {
        a: Option<String>,
        b: Option<String>,
    },
    HeaderChanged {
        names: Vec<String>,
    },
    ParamChanged {
        names: Vec<String>,
    },
    TtlExpired {
        gap_secs: i64,
        ttl_secs: u64,
        since_seq: u64,
    },
    Unexplained {
        invisible: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    Extends,
    PrefixOf,
    Identical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TokenMethod {
    Anchor { seq: u64 },
    Bytes,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Explanation {
    pub servable_bytes: usize,
    pub servable_tokens: Option<u64>,
    pub method: Option<TokenMethod>,
    pub cache_read: Option<u64>,
    pub verdict: Verdict,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DiffView {
    pub a: Option<u64>,
    pub b: u64,
    pub base_picked: bool,
    pub facts: Vec<FactDiff>,
    pub headers: Vec<HeaderDiff>,
    pub shared_segments: usize,
    pub divergence: Option<Divergence>,
    pub removed: Vec<String>,
    pub added: Vec<String>,
    pub explanation: Explanation,
}

pub struct DiffInput<'a> {
    pub a: Option<&'a CallRecord>,
    pub b: &'a CallRecord,
    pub session: &'a [CallRecord],
    pub provider: &'a dyn Provider,
    pub base_picked: bool,
}

struct Pair<'a> {
    a: &'a CallRecord,
    b: &'a CallRecord,
    session: &'a [CallRecord],
    provider: &'a dyn Provider,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffError {
    NotInOrder { a: u64, b: u64 },
    NotInference { seq: u64 },
}

impl std::fmt::Display for DiffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiffError::NotInOrder { a, b } => {
                write!(f, "call A must come before call B (got {a} -> {b})")
            }
            DiffError::NotInference { seq } => write!(f, "call {seq} is not an inference call"),
        }
    }
}

impl std::error::Error for DiffError {}

pub fn check_pair(a: Option<&CallRecord>, b: &CallRecord) -> Result<(), DiffError> {
    if b.kind != CallKind::Inference {
        return Err(DiffError::NotInference { seq: b.seq });
    }
    let Some(a) = a else {
        return Ok(());
    };
    if a.kind != CallKind::Inference {
        return Err(DiffError::NotInference { seq: a.seq });
    }
    if a.seq >= b.seq {
        return Err(DiffError::NotInOrder { a: a.seq, b: b.seq });
    }
    Ok(())
}

fn segments_of(call: &CallRecord, provider: &dyn Provider) -> Option<Vec<PromptSegment>> {
    match &call.request_body {
        RequestBody::Json(body) => Some(provider.segments(body)),
        RequestBody::Base64(_) => None,
    }
}

fn shared_prefix(a: &[PromptSegment], b: &[PromptSegment]) -> usize {
    a.iter()
        .zip(b)
        .take_while(|(left, right)| left.content == right.content)
        .count()
}

pub fn pick_base<'a>(
    calls: &'a [CallRecord],
    seq: u64,
    session: &str,
    provider: &dyn Provider,
) -> Result<Option<&'a CallRecord>, ReadError> {
    let target =
        calls
            .iter()
            .find(|call| call.seq == seq)
            .ok_or_else(|| ReadError::UnknownCall {
                seq,
                session: session.to_string(),
            })?;
    let Some(target_segments) = segments_of(target, provider) else {
        return Ok(None);
    };
    Ok(calls
        .iter()
        .filter(|call| call.seq < seq && call.kind == CallKind::Inference)
        .filter_map(|call| {
            let shared = shared_prefix(&segments_of(call, provider)?, &target_segments);
            Some((shared, call))
        })
        .filter(|(shared, _)| *shared > 0)
        .max_by_key(|(shared, call)| (*shared, call.seq))
        .map(|(_, call)| call))
}

fn header_value<'a>(call: &'a CallRecord, name: &str) -> Option<&'a str> {
    call.request_headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn fact_diffs(a: &CallRecord, b: &CallRecord) -> Vec<FactDiff> {
    let model = (a.facts.model != b.facts.model).then(|| FactDiff {
        name: "model".to_string(),
        a: a.facts.model.clone().map(Value::String),
        b: b.facts.model.clone().map(Value::String),
    });
    let keys: BTreeSet<&String> = a.facts.params.keys().chain(b.facts.params.keys()).collect();
    let params = keys.into_iter().filter_map(|key| {
        let (left, right) = (a.facts.params.get(key), b.facts.params.get(key));
        (left != right).then(|| FactDiff {
            name: key.clone(),
            a: left.cloned(),
            b: right.cloned(),
        })
    });
    model.into_iter().chain(params).collect()
}

fn header_diffs(a: &CallRecord, b: &CallRecord, provider: &dyn Provider) -> Vec<HeaderDiff> {
    provider
        .cache_relevant_headers()
        .iter()
        .filter_map(|name| {
            let (left, right) = (header_value(a, name), header_value(b, name));
            (left != right).then(|| HeaderDiff {
                name: name.to_string(),
                a: left.map(str::to_string),
                b: right.map(str::to_string),
            })
        })
        .collect()
}

fn floor_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn excerpt(text: &str, offset: usize) -> String {
    let start = floor_boundary(text, offset);
    let end = floor_boundary(text, start + EXCERPT_BYTES);
    text[start..end].to_string()
}

fn relation(a_len: usize, b_len: usize, shared: usize) -> Relation {
    if b_len > shared {
        Relation::Extends
    } else if a_len > shared {
        Relation::PrefixOf
    } else {
        Relation::Identical
    }
}

fn divergence(shared: usize, a: &[PromptSegment], b: &[PromptSegment]) -> Option<Divergence> {
    let (left, right) = (a.get(shared)?, b.get(shared)?);
    let offset = left
        .content
        .bytes()
        .zip(right.content.bytes())
        .position(|(x, y)| x != y)
        .unwrap_or_else(|| left.content.len().min(right.content.len()));
    Some(Divergence {
        index: shared,
        label_a: left.label.clone(),
        label_b: right.label.clone(),
        offset,
        a_excerpt: excerpt(&left.content, offset),
        b_excerpt: excerpt(&right.content, offset),
    })
}

fn labels_after(segments: &[PromptSegment], from: usize) -> Vec<String> {
    segments
        .iter()
        .skip(from)
        .map(|segment| segment.label.clone())
        .collect()
}

fn prompt_tokens(usage: &Usage) -> u64 {
    usage.input_uncached
        + usage.cache_read
        + usage
            .cache_writes
            .iter()
            .map(|write| write.tokens)
            .sum::<u64>()
}

fn counted_usage(call: &CallRecord) -> Option<&Usage> {
    match &call.usage {
        UsageOutcome::Reported(usage) | UsageOutcome::Partial(usage) => Some(usage),
        UsageOutcome::Missing => None,
    }
}

fn last_touch<'a>(
    input: &Pair<'a>,
    through: usize,
    b_segments: &[PromptSegment],
) -> &'a CallRecord {
    input
        .session
        .iter()
        .filter(|call| call.seq >= input.a.seq && call.seq < input.b.seq)
        .filter(|call| call.kind == CallKind::Inference)
        .filter(|call| {
            segments_of(call, input.provider).is_some_and(|segments| {
                segments.len() > through
                    && segments[..=through]
                        .iter()
                        .zip(&b_segments[..=through])
                        .all(|(left, right)| left.text == right.text)
            })
        })
        .max_by_key(|call| call.seq)
        .unwrap_or(input.a)
}

fn last_breakpoint(segments: &[PromptSegment]) -> Option<usize> {
    segments
        .iter()
        .rposition(|segment| segment.breakpoint.is_some())
}

fn anchor_tokens<'a>(
    input: &Pair<'a>,
    index: usize,
    b_segments: &[PromptSegment],
) -> Option<(&'a CallRecord, u64)> {
    input
        .session
        .iter()
        .filter(|call| call.seq <= input.b.seq && call.kind == CallKind::Inference)
        .filter_map(|call| {
            let usage = counted_usage(call)?;
            let segments = segments_of(call, input.provider)?;
            let identical = segments.len() > index
                && segments[..=index]
                    .iter()
                    .zip(&b_segments[..=index])
                    .all(|(left, right)| left.content == right.content);
            let anchored = identical && last_breakpoint(&segments) == Some(index);
            anchored.then(|| {
                let written: u64 = usage.cache_writes.iter().map(|write| write.tokens).sum();
                (call, usage.cache_read + written)
            })
        })
        .max_by_key(|(call, _)| call.seq)
}

struct Breakpoint {
    index: usize,
    tier: CacheTier,
}

fn mismatch_verdict(
    input: &Pair,
    facts: &[FactDiff],
    headers: &[HeaderDiff],
    breakpoint: &Breakpoint,
    b_segments: &[PromptSegment],
) -> Verdict {
    if input.a.facts.model != input.b.facts.model {
        return Verdict::ModelChanged {
            a: input.a.facts.model.clone(),
            b: input.b.facts.model.clone(),
        };
    }
    if !headers.is_empty() {
        return Verdict::HeaderChanged {
            names: headers.iter().map(|h| h.name.clone()).collect(),
        };
    }
    let relevant: Vec<String> = facts
        .iter()
        .filter(|fact| {
            input
                .provider
                .cache_relevant_params()
                .contains(&fact.name.as_str())
        })
        .map(|fact| fact.name.clone())
        .collect();
    if !relevant.is_empty() {
        return Verdict::ParamChanged { names: relevant };
    }
    if let CacheTier::Ttl { seconds } = breakpoint.tier {
        let touch = last_touch(input, breakpoint.index, b_segments);
        let gap = (input.b.started_at - touch.ended_at).num_seconds();
        if gap > seconds as i64 {
            return Verdict::TtlExpired {
                gap_secs: gap,
                ttl_secs: seconds,
                since_seq: touch.seq,
            };
        }
    }
    Verdict::Unexplained {
        invisible: INVISIBLE_CAUSES.iter().map(|c| c.to_string()).collect(),
    }
}

fn explain(
    input: &Pair,
    base: &DiffView,
    b_segments: &[PromptSegment],
    shared: usize,
    served: Verdict,
) -> Explanation {
    let usage = counted_usage(input.b);
    let cache_read = usage.map(|u| u.cache_read);
    let Some(breakpoint) = b_segments[..shared]
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, segment)| segment.breakpoint.map(|tier| Breakpoint { index, tier }))
    else {
        return Explanation {
            servable_bytes: 0,
            servable_tokens: None,
            method: None,
            cache_read,
            verdict: Verdict::NoSharedBreakpoint,
        };
    };
    let servable_bytes: usize = b_segments[..=breakpoint.index]
        .iter()
        .map(|s| s.text.len())
        .sum();
    let Some(usage) = usage else {
        return Explanation {
            servable_bytes,
            servable_tokens: None,
            method: None,
            cache_read: None,
            verdict: Verdict::UsageMissing,
        };
    };
    let (servable, method) = match anchor_tokens(input, breakpoint.index, b_segments) {
        Some((call, tokens)) => (tokens, TokenMethod::Anchor { seq: call.seq }),
        None => {
            let total: usize = b_segments.iter().map(|s| s.text.len()).sum();
            let estimated = if total == 0 {
                0
            } else {
                (servable_bytes as f64 * prompt_tokens(usage) as f64 / total as f64).round() as u64
            };
            (estimated, TokenMethod::Bytes)
        }
    };
    let verdict = if usage.cache_read as f64 >= servable as f64 * CACHE_AGREE_RATIO {
        served
    } else {
        mismatch_verdict(input, &base.facts, &base.headers, &breakpoint, b_segments)
    };
    Explanation {
        servable_bytes,
        servable_tokens: Some(servable),
        method: Some(method),
        cache_read,
        verdict,
    }
}

fn empty_view(input: &DiffInput, verdict: Verdict, cache_read: Option<u64>) -> DiffView {
    DiffView {
        a: input.a.map(|a| a.seq),
        b: input.b.seq,
        base_picked: input.base_picked,
        facts: Vec::new(),
        headers: Vec::new(),
        shared_segments: 0,
        divergence: None,
        removed: Vec::new(),
        added: Vec::new(),
        explanation: Explanation {
            servable_bytes: 0,
            servable_tokens: None,
            method: None,
            cache_read,
            verdict,
        },
    }
}

pub fn diff_view(input: &DiffInput) -> DiffView {
    let cache_read = counted_usage(input.b).map(|usage| usage.cache_read);
    let Some(a) = input.a else {
        let verdict = match segments_of(input.b, input.provider) {
            Some(_) => Verdict::NoBase,
            None => Verdict::NoSegments,
        };
        return empty_view(input, verdict, cache_read);
    };
    let pair = Pair {
        a,
        b: input.b,
        session: input.session,
        provider: input.provider,
    };
    let facts = fact_diffs(a, input.b);
    let headers = header_diffs(a, input.b, input.provider);
    let base = DiffView {
        facts,
        headers,
        ..empty_view(input, Verdict::NoSegments, cache_read)
    };
    let (Some(a_segments), Some(b_segments)) = (
        segments_of(a, input.provider),
        segments_of(input.b, input.provider),
    ) else {
        return base;
    };
    let shared = shared_prefix(&a_segments, &b_segments);
    let div = divergence(shared, &a_segments, &b_segments);
    let skip = shared + usize::from(div.is_some());
    let served = match &div {
        Some(d) => Verdict::PrefixChanged {
            label: d.label_b.clone(),
        },
        None => Verdict::PrefixShared {
            relation: relation(a_segments.len(), b_segments.len(), shared),
        },
    };
    let explanation = explain(&pair, &base, &b_segments, shared, served);
    DiffView {
        shared_segments: shared,
        removed: labels_after(&a_segments, skip),
        added: labels_after(&b_segments, skip),
        divergence: div,
        explanation,
        ..base
    }
}

fn value_text(value: &Option<Value>) -> String {
    value
        .as_ref()
        .map_or_else(|| "-".to_string(), Value::to_string)
}

fn verdict_text(view: &DiffView) -> String {
    let verdict = &view.explanation.verdict;
    let cache_read = view.explanation.cache_read;
    let model = |value: &Option<String>| value.clone().unwrap_or_else(|| "-".to_string());
    match verdict {
        Verdict::NoSegments => "request body is not JSON on one side; no segments".to_string(),
        Verdict::NoBase if cache_read.unwrap_or(0) > 0 => {
            "no earlier call in this session shares a prompt prefix; the cache read came from outside this session".to_string()
        }
        Verdict::NoBase => {
            "cold write: no earlier call in this session shares a prompt prefix".to_string()
        }
        Verdict::NoSharedBreakpoint => "no cache breakpoint in the shared prefix".to_string(),
        Verdict::UsageMissing => "call b has no usage; cannot compare".to_string(),
        Verdict::PrefixChanged { label } => {
            format!("cache served the shared prefix; prompts diverge at {label}")
        }
        Verdict::PrefixShared { relation } => {
            let relation = match relation {
                Relation::Extends => "b extends a",
                Relation::PrefixOf => "b is a prefix of a",
                Relation::Identical => "prompts are identical",
            };
            format!("cache served the shared prefix; {relation}")
        }
        Verdict::ModelChanged { a, b } => {
            format!("model changed {} -> {}", model(a), model(b))
        }
        Verdict::HeaderChanged { names } => {
            format!("cache-relevant header changed: {}", names.join(", "))
        }
        Verdict::ParamChanged { names } => {
            format!("cache-relevant param changed: {}", names.join(", "))
        }
        Verdict::TtlExpired {
            gap_secs,
            ttl_secs,
            since_seq,
        } => format!("ttl expired: {gap_secs}s since call {since_seq} exceeds {ttl_secs}s"),
        Verdict::Unexplained { invisible } => {
            format!("unexplained; invisible causes: {}", invisible.join("; "))
        }
    }
}

pub fn diff_table(view: &DiffView) -> String {
    let picked = match (view.a, &view.explanation.verdict, view.base_picked) {
        (None, Verdict::NoBase, _) => " (no earlier call shares a prompt prefix)",
        (None, _, _) => " (request body is not JSON; no base picked)",
        (Some(_), _, true) => " (base picked by longest shared prefix)",
        (Some(_), _, false) => "",
    };
    let a = view
        .a
        .map_or_else(|| "-".to_string(), |seq| seq.to_string());
    let mut lines = vec![format!("diff {a} -> {}{picked}", view.b)];
    lines.extend(view.facts.iter().map(|fact| {
        format!(
            "fact {}: {} -> {}",
            fact.name,
            value_text(&fact.a),
            value_text(&fact.b)
        )
    }));
    lines.extend(view.headers.iter().map(|header| {
        format!(
            "header {}: {} -> {}",
            header.name,
            header.a.as_deref().unwrap_or("-"),
            header.b.as_deref().unwrap_or("-")
        )
    }));
    lines.push(format!("shared segments: {}", view.shared_segments));
    if let Some(div) = &view.divergence {
        lines.push(format!(
            "diverges at segment {}: {} -> {}, byte offset {}",
            div.index, div.label_a, div.label_b, div.offset
        ));
        lines.push(format!("  a: {:?}", div.a_excerpt));
        lines.push(format!("  b: {:?}", div.b_excerpt));
    }
    if !view.removed.is_empty() {
        lines.push(format!("removed: {}", view.removed.join(", ")));
    }
    if !view.added.is_empty() {
        lines.push(format!("added: {}", view.added.join(", ")));
    }
    let explanation = &view.explanation;
    let cache_read = explanation
        .cache_read
        .map_or_else(|| "-".to_string(), |v| v.to_string());
    let estimate = match (explanation.servable_tokens, explanation.method) {
        (Some(tokens), Some(TokenMethod::Anchor { seq })) => {
            format!("servable {tokens} tokens exact from call {seq}, ")
        }
        (Some(tokens), _) => format!(
            "servable ~{tokens} tokens by byte estimate from {} bytes, ",
            explanation.servable_bytes
        ),
        (None, _) => String::new(),
    };
    lines.push(format!(
        "cache: {} ({estimate}cache_read {cache_read}; agree threshold {:.0}%)",
        verdict_text(view),
        CACHE_AGREE_RATIO * 100.0
    ));
    lines.join("\n")
}

#[cfg(test)]
mod tests;
