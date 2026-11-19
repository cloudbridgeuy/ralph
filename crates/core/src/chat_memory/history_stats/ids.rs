use super::parse::{Call, Tool};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub scope: Option<String>,
    pub lo: u64,
    pub hi: u64,
}

pub fn normalize_scope(name: &str) -> Option<String> {
    match name {
        "" | "main" | "own" => None,
        other => Some(other.to_string()),
    }
}

fn split_prefix(text: &str) -> (Option<&str>, &str) {
    match text.split_once(':') {
        Some((scope, rest)) => (Some(scope), rest),
        None => (None, text),
    }
}

fn parse_bounds(text: &str) -> Option<(u64, u64)> {
    let (lo, hi) = match text.split_once('-') {
        Some((lo, hi)) => (lo.parse().ok()?, hi.parse().ok()?),
        None => {
            let only = text.parse().ok()?;
            (only, only)
        }
    };
    (lo <= hi).then_some((lo, hi))
}

pub fn parse_span(text: &str, default_scope: Option<&str>) -> Option<Span> {
    let (prefix, rest) = split_prefix(text.trim());
    let (lo, hi) = parse_bounds(rest)?;
    let scope = prefix.or(default_scope).and_then(normalize_scope);
    Some(Span { scope, lo, hi })
}

pub fn view_low(text: &str, scope: Option<&str>) -> Option<u64> {
    let (prefix, rest) = split_prefix(text.trim());
    if let Some(prefix) = prefix {
        if normalize_scope(prefix).as_deref() != scope {
            return None;
        }
    }
    parse_bounds(rest).map(|(lo, _)| lo)
}

fn index_arg(value: Option<&Value>) -> Option<u64> {
    match value? {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

pub fn call_span(call: &Call) -> Option<Span> {
    let scope = call.scope.as_deref();
    let id = || call.args.get("id").and_then(Value::as_str);
    match call.tool {
        Tool::Zoom => parse_span(id()?, scope),
        Tool::Read => match index_arg(call.args.get("from")) {
            Some(lo) => {
                let hi = index_arg(call.args.get("to")).unwrap_or(lo);
                let scope = scope.and_then(normalize_scope);
                (lo <= hi).then_some(Span { scope, lo, hi })
            }
            None => parse_span(id()?, scope),
        },
        Tool::Recall | Tool::Search | Tool::Unknown => None,
    }
}

pub fn overlaps(left: &Span, right: &Span) -> bool {
    left.scope == right.scope && left.lo <= right.hi && right.lo <= left.hi
}
