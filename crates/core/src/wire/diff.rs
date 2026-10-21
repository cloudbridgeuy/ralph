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
    NoSharedBreakpoint,
    UsageMissing,
    PrefixChanged {
        label: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Explanation {
    pub servable_bytes: usize,
    pub estimated_tokens: Option<u64>,
    pub cache_read: Option<u64>,
    pub verdict: Verdict,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DiffView {
    pub a: u64,
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
    pub a: &'a CallRecord,
    pub b: &'a CallRecord,
    pub session: &'a [CallRecord],
    pub provider: &'a dyn Provider,
    pub base_picked: bool,
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
        .take_while(|(left, right)| left.text == right.text)
        .count()
}

pub fn pick_base<'a>(
    calls: &'a [CallRecord],
    seq: u64,
    session: &str,
    provider: &dyn Provider,
) -> Result<&'a CallRecord, ReadError> {
    let no_base = || ReadError::NoBase {
        seq,
        session: session.to_string(),
    };
    let target =
        calls
            .iter()
            .find(|call| call.seq == seq)
            .ok_or_else(|| ReadError::UnknownCall {
                seq,
                session: session.to_string(),
            })?;
    let target_segments = segments_of(target, provider).ok_or_else(no_base)?;
    calls
        .iter()
        .filter(|call| call.seq < seq && call.kind == CallKind::Inference)
        .filter_map(|call| {
            let shared = shared_prefix(&segments_of(call, provider)?, &target_segments);
            Some((shared, call))
        })
        .filter(|(shared, _)| *shared > 0)
        .max_by_key(|(shared, call)| (*shared, call.seq))
        .map(|(_, call)| call)
        .ok_or_else(no_base)
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

fn divergence(shared: usize, a: &[PromptSegment], b: &[PromptSegment]) -> Option<Divergence> {
    let (left, right) = (a.get(shared)?, b.get(shared)?);
    let offset = left
        .text
        .bytes()
        .zip(right.text.bytes())
        .position(|(x, y)| x != y)
        .unwrap_or_else(|| left.text.len().min(right.text.len()));
    Some(Divergence {
        index: shared,
        label_a: left.label.clone(),
        label_b: right.label.clone(),
        offset,
        a_excerpt: excerpt(&left.text, offset),
        b_excerpt: excerpt(&right.text, offset),
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
    input: &DiffInput<'a>,
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

struct Breakpoint {
    index: usize,
    tier: CacheTier,
}

fn mismatch_verdict(
    input: &DiffInput,
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
    input: &DiffInput,
    base: &DiffView,
    b_segments: &[PromptSegment],
    shared: usize,
    div: Option<&Divergence>,
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
            estimated_tokens: None,
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
            estimated_tokens: None,
            cache_read: None,
            verdict: Verdict::UsageMissing,
        };
    };
    let total: usize = b_segments.iter().map(|s| s.text.len()).sum();
    let estimated = if total == 0 {
        0
    } else {
        (servable_bytes as f64 * prompt_tokens(usage) as f64 / total as f64).round() as u64
    };
    let verdict = if usage.cache_read as f64 >= estimated as f64 * CACHE_AGREE_RATIO {
        Verdict::PrefixChanged {
            label: div.map(|d| d.label_b.clone()),
        }
    } else {
        mismatch_verdict(input, &base.facts, &base.headers, &breakpoint, b_segments)
    };
    Explanation {
        servable_bytes,
        estimated_tokens: Some(estimated),
        cache_read,
        verdict,
    }
}

pub fn diff_view(input: &DiffInput) -> DiffView {
    let facts = fact_diffs(input.a, input.b);
    let headers = header_diffs(input.a, input.b, input.provider);
    let base = DiffView {
        a: input.a.seq,
        b: input.b.seq,
        base_picked: input.base_picked,
        facts: facts.clone(),
        headers: headers.clone(),
        shared_segments: 0,
        divergence: None,
        removed: Vec::new(),
        added: Vec::new(),
        explanation: Explanation {
            servable_bytes: 0,
            estimated_tokens: None,
            cache_read: None,
            verdict: Verdict::NoSegments,
        },
    };
    let (Some(a_segments), Some(b_segments)) = (
        segments_of(input.a, input.provider),
        segments_of(input.b, input.provider),
    ) else {
        return base;
    };
    let shared = shared_prefix(&a_segments, &b_segments);
    let div = divergence(shared, &a_segments, &b_segments);
    let skip = shared + usize::from(div.is_some());
    let explanation = explain(input, &base, &b_segments, shared, div.as_ref());
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

fn verdict_text(verdict: &Verdict) -> String {
    let model = |value: &Option<String>| value.clone().unwrap_or_else(|| "-".to_string());
    match verdict {
        Verdict::NoSegments => "request body is not JSON on one side; no segments".to_string(),
        Verdict::NoSharedBreakpoint => "no cache breakpoint in the shared prefix".to_string(),
        Verdict::UsageMissing => "call b has no usage; cannot compare".to_string(),
        Verdict::PrefixChanged { label: Some(label) } => {
            format!("cache served the shared prefix; prompts diverge at {label}")
        }
        Verdict::PrefixChanged { label: None } => {
            "cache served the shared prefix; b extends a".to_string()
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
    let picked = if view.base_picked {
        " (base picked by longest shared prefix)"
    } else {
        ""
    };
    let mut lines = vec![format!("diff {} -> {}{}", view.a, view.b, picked)];
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
    let estimate = explanation
        .estimated_tokens
        .map_or_else(String::new, |tokens| {
            format!(
                "estimated servable ~{tokens} tokens from {} bytes, ",
                explanation.servable_bytes
            )
        });
    lines.push(format!(
        "cache: {} ({estimate}cache_read {cache_read}; agree threshold {:.0}%)",
        verdict_text(&explanation.verdict),
        CACHE_AGREE_RATIO * 100.0
    ));
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::provider::anthropic::Anthropic;
    use crate::wire::provider::RequestFacts;
    use crate::wire::usage::CacheWrite;
    use chrono::{DateTime, TimeZone, Utc};
    use serde_json::json;

    fn t(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
    }

    fn blk(text: &str, ttl: Option<&str>) -> Value {
        match ttl {
            Some(ttl) => {
                json!({"type": "text", "text": text, "cache_control": {"type": "ephemeral", "ttl": ttl}})
            }
            None => json!({"type": "text", "text": text}),
        }
    }

    fn usage(uncached: u64, read: u64) -> UsageOutcome {
        UsageOutcome::Reported(Usage {
            input_uncached: uncached,
            cache_read: read,
            cache_writes: vec![],
            output: 1,
            reasoning: None,
            other: vec![],
        })
    }

    fn call(seq: u64, start: i64, end: i64, blocks: Vec<Value>, usage: UsageOutcome) -> CallRecord {
        CallRecord {
            seq,
            started_at: t(start),
            first_byte_at: None,
            ended_at: t(end),
            method: "POST".into(),
            path: "/v1/messages".into(),
            status: Some(200),
            kind: CallKind::Inference,
            facts: RequestFacts {
                model: Some("m".into()),
                params: serde_json::Map::new(),
            },
            request_headers: vec![],
            response_headers: vec![],
            request_body: RequestBody::Json(
                json!({"messages": [{"role": "user", "content": blocks}]}),
            ),
            raw_usage: None,
            usage,
            input_equivalent: None,
            complete: true,
            error: None,
            response_events: None,
        }
    }

    fn pair(a_tail: &str, b_tail: &str, b_usage: UsageOutcome) -> (CallRecord, CallRecord) {
        let head = || blk("shared head", Some("5m"));
        (
            call(1, 0, 10, vec![head(), blk(a_tail, None)], usage(5, 0)),
            call(2, 20, 30, vec![head(), blk(b_tail, None)], b_usage),
        )
    }

    fn view_of(a: &CallRecord, b: &CallRecord, session: &[CallRecord]) -> DiffView {
        diff_view(&DiffInput {
            a,
            b,
            session,
            provider: &Anthropic,
            base_picked: false,
        })
    }

    fn verdict_of(a: &CallRecord, b: &CallRecord) -> Verdict {
        view_of(a, b, &[a.clone(), b.clone()]).explanation.verdict
    }

    #[test]
    fn pick_base_prefers_longest_prefix_over_predecessor() {
        let x = |tail: &str| vec![blk("x1", None), blk("x2", None), blk(tail, None)];
        let mut other = call(2, 5, 6, vec![blk("y1", None)], usage(1, 0));
        other.facts.model = Some("other".into());
        let calls = vec![
            call(1, 0, 1, x("x3"), usage(1, 0)),
            other,
            call(3, 10, 11, x("x4"), usage(1, 0)),
        ];
        assert_eq!(pick_base(&calls, 3, "s", &Anthropic).unwrap().seq, 1);
    }

    #[test]
    fn pick_base_tie_takes_highest_seq() {
        let body = || vec![blk("x1", None)];
        let calls = vec![
            call(1, 0, 1, body(), usage(1, 0)),
            call(2, 2, 3, body(), usage(1, 0)),
            call(3, 4, 5, body(), usage(1, 0)),
        ];
        assert_eq!(pick_base(&calls, 3, "s", &Anthropic).unwrap().seq, 2);
    }

    #[test]
    fn pick_base_without_shared_prefix_errors() {
        let calls = vec![
            call(1, 0, 1, vec![blk("p", None)], usage(1, 0)),
            call(2, 2, 3, vec![blk("q", None)], usage(1, 0)),
        ];
        let error = pick_base(&calls, 2, "s", &Anthropic).unwrap_err();
        assert_eq!(
            error.to_string(),
            "no earlier call in session s shares a prompt prefix with call 2"
        );
    }

    #[test]
    fn facts_and_cache_headers_differ_case_insensitively() {
        let (mut a, mut b) = pair("t", "t", usage(1, 0));
        b.facts.model = Some("n".into());
        a.facts.params.insert("max_tokens".into(), json!(1));
        b.facts.params.insert("max_tokens".into(), json!(2));
        b.facts.params.insert("thinking".into(), json!({"a": 1}));
        a.request_headers = vec![
            ("Anthropic-Beta".into(), "x".into()),
            ("anthropic-version".into(), "1".into()),
            ("user-agent".into(), "p".into()),
        ];
        b.request_headers = vec![
            ("anthropic-beta".into(), "x".into()),
            ("Anthropic-Version".into(), "2".into()),
            ("user-agent".into(), "q".into()),
        ];
        let view = view_of(&a, &b, &[]);
        let names: Vec<&str> = view.facts.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["model", "max_tokens", "thinking"]);
        assert_eq!(view.facts[2].a, None);
        assert_eq!(
            view.headers,
            [HeaderDiff {
                name: "anthropic-version".into(),
                a: Some("1".into()),
                b: Some("2".into())
            }]
        );
    }

    #[test]
    fn divergence_reports_offset_excerpt_and_removed_added() {
        let (mut a, b) = pair("same prefix then A", "same prefix then B", usage(1, 0));
        if let RequestBody::Json(body) = &mut a.request_body {
            body["messages"][0]["content"]
                .as_array_mut()
                .unwrap()
                .push(blk("tail", None));
        }
        let view = view_of(&a, &b, &[]);
        let div = view.divergence.unwrap();
        assert_eq!(view.shared_segments, 1);
        assert_eq!(div.index, 1);
        assert_eq!(div.label_a, "msg[0].1 user/text");
        let text = segments_of(&b, &Anthropic).unwrap()[1].text.clone();
        assert_eq!(&text[div.offset..div.offset + 1], "B");
        assert_eq!(div.b_excerpt, text[div.offset..]);
        assert!(div.a_excerpt.starts_with('A'));
        assert_eq!(view.removed, ["msg[0].2 user/text"]);
        assert!(view.added.is_empty());
    }

    #[test]
    fn excerpt_snaps_to_char_boundaries_and_caps_length() {
        let long = "z".repeat(100);
        let (a, b) = pair(
            &format!("\u{e9}{long}"),
            &format!("\u{eb}{long}"),
            usage(1, 0),
        );
        let div = view_of(&a, &b, &[]).divergence.unwrap();
        assert!(div.a_excerpt.starts_with('\u{e9}'));
        assert!(div.b_excerpt.starts_with('\u{eb}'));
        assert!(div.a_excerpt.len() <= EXCERPT_BYTES);
        assert!(excerpt("\u{e9}\u{e9}\u{e9}", 3).len() <= EXCERPT_BYTES);
    }

    #[test]
    fn b_extends_a_has_no_divergence_and_only_added() {
        let head = || blk("h", Some("5m"));
        let a = call(1, 0, 10, vec![head()], usage(1, 0));
        let b = call(2, 20, 30, vec![head(), blk("more", None)], usage(0, 100));
        let view = view_of(&a, &b, &[]);
        assert_eq!(view.divergence, None);
        assert_eq!(view.shared_segments, 1);
        assert!(view.removed.is_empty());
        assert_eq!(view.added, ["msg[0].1 user/text"]);
        assert_eq!(
            view.explanation.verdict,
            Verdict::PrefixChanged { label: None }
        );
    }

    #[test]
    fn string_content_shares_segment_with_block_content() {
        let mut a = call(1, 0, 10, vec![], usage(1, 0));
        a.request_body = RequestBody::Json(
            json!({"messages": [{"role": "user", "content": "same"}, {"role": "user", "content": "x"}]}),
        );
        let b = call(
            2,
            20,
            30,
            vec![blk("same", None), blk("y", None)],
            usage(1, 0),
        );
        let b_text = segments_of(&b, &Anthropic).unwrap();
        assert_eq!(segments_of(&a, &Anthropic).unwrap()[0], b_text[0]);
        assert_eq!(view_of(&a, &b, &[]).shared_segments, 1);
    }

    #[test]
    fn no_shared_breakpoint() {
        let a = call(1, 0, 10, vec![blk("p", None)], usage(1, 0));
        let b = call(2, 20, 30, vec![blk("p", None)], usage(1, 7));
        let explanation = view_of(&a, &b, &[]).explanation;
        assert_eq!(explanation.verdict, Verdict::NoSharedBreakpoint);
        assert_eq!(explanation.cache_read, Some(7));
        assert_eq!(explanation.servable_bytes, 0);
    }

    #[test]
    fn cache_agreeing_with_estimate_means_prefix_changed() {
        let (a, b) = pair("a", "b", usage(0, 100));
        let view = view_of(&a, &b, &[]);
        assert_eq!(
            view.explanation.verdict,
            Verdict::PrefixChanged {
                label: Some("msg[0].1 user/text".into())
            }
        );
        assert!(view.explanation.estimated_tokens.unwrap() > 0);
    }

    #[test]
    fn model_header_param_verdicts_in_order() {
        let (mut a, mut b) = pair("a", "b", usage(5, 0));
        a.facts.params.insert("thinking".into(), json!(1));
        assert_eq!(
            verdict_of(&a, &b),
            Verdict::ParamChanged {
                names: vec!["thinking".into()]
            }
        );
        b.request_headers = vec![("anthropic-beta".into(), "x".into())];
        assert_eq!(
            verdict_of(&a, &b),
            Verdict::HeaderChanged {
                names: vec!["anthropic-beta".into()]
            }
        );
        b.facts.model = Some("n".into());
        assert_eq!(
            verdict_of(&a, &b),
            Verdict::ModelChanged {
                a: Some("m".into()),
                b: Some("n".into())
            }
        );
    }

    #[test]
    fn irrelevant_param_does_not_explain() {
        let (mut a, b) = pair("a", "b", usage(5, 0));
        a.facts.params.insert("max_tokens".into(), json!(1));
        assert!(matches!(verdict_of(&a, &b), Verdict::Unexplained { .. }));
    }

    #[test]
    fn ttl_gap_beyond_ttl_expires() {
        let (a, mut b) = pair("a", "b", usage(5, 0));
        b.started_at = t(400);
        assert_eq!(
            verdict_of(&a, &b),
            Verdict::TtlExpired {
                gap_secs: 390,
                ttl_secs: 300,
                since_seq: 1
            }
        );
    }

    #[test]
    fn intermediate_refresh_prevents_ttl_expiry() {
        let (a, mut b) = pair("a", "b", usage(5, 0));
        b.seq = 3;
        b.started_at = t(400);
        let refresh = call(
            2,
            200,
            350,
            vec![blk("shared head", Some("5m")), blk("mid", None)],
            usage(1, 1),
        );
        let session = [a.clone(), refresh, b.clone()];
        assert!(matches!(
            view_of(&a, &b, &session).explanation.verdict,
            Verdict::Unexplained { .. }
        ));
    }

    #[test]
    fn small_gap_is_unexplained_with_invisible_causes() {
        let (a, b) = pair("a", "b", usage(5, 0));
        match verdict_of(&a, &b) {
            Verdict::Unexplained { invisible } => assert_eq!(invisible.len(), 3),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn missing_usage_and_non_json_bodies() {
        let (a, mut b) = pair("a", "b", UsageOutcome::Missing);
        let explanation = view_of(&a, &b, &[]).explanation;
        assert_eq!(explanation.verdict, Verdict::UsageMissing);
        assert_eq!(
            (explanation.estimated_tokens, explanation.cache_read),
            (None, None)
        );
        b.request_body = RequestBody::Base64(String::new());
        let view = view_of(&a, &b, &[]);
        assert_eq!(view.explanation.verdict, Verdict::NoSegments);
        assert_eq!(view.shared_segments, 0);
    }

    #[test]
    fn partial_usage_counts_like_reported() {
        let (a, reported) = pair("a", "b", usage(5, 20));
        let mut partial = reported.clone();
        if let UsageOutcome::Reported(u) = reported.usage.clone() {
            partial.usage = UsageOutcome::Partial(u);
        }
        assert_eq!(
            view_of(&a, &reported, &[]).explanation,
            view_of(&a, &partial, &[]).explanation
        );
    }

    #[test]
    fn cache_writes_count_toward_prompt_tokens() {
        let (a, mut b) = pair("a", "b", usage(0, 0));
        b.usage = UsageOutcome::Reported(Usage {
            input_uncached: 0,
            cache_read: 0,
            cache_writes: vec![CacheWrite {
                tier: CacheTier::Unspecified,
                tokens: 100,
            }],
            output: 1,
            reasoning: None,
            other: vec![],
        });
        assert!(view_of(&a, &b, &[]).explanation.estimated_tokens.unwrap() > 0);
    }

    #[test]
    fn table_names_verdict_and_escapes_excerpts() {
        let (a, mut b) = pair("a\nb", "c", usage(5, 0));
        b.started_at = t(400);
        let table = diff_table(&view_of(&a, &b, &[]));
        assert!(table.starts_with("diff 1 -> 2\n"), "{table}");
        assert!(table.contains("shared segments: 1"), "{table}");
        assert!(table.contains("\\n"), "{table}");
        assert!(table.contains("cache: ttl expired"), "{table}");
        assert!(table.contains("agree threshold 90%"), "{table}");
        let none = Explanation {
            estimated_tokens: None,
            cache_read: None,
            ..view_of(&a, &b, &[]).explanation
        };
        let bare = DiffView {
            explanation: none,
            ..view_of(&a, &b, &[])
        };
        assert!(diff_table(&bare).ends_with("(cache_read -; agree threshold 90%)"));
    }
}
