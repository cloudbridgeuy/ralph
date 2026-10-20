use super::query::Query;
use crate::chat_memory::index::fnv1a;
use crate::chat_memory::{Entry, Role};
use crate::mcp::string_arg;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde_json::Value;

pub const DEFAULT_LIMIT: usize = 20;
pub const MAX_LIMIT: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchCursor {
    pub(super) offset: usize,
    pub(super) count: usize,
    pub(super) fingerprint: u64,
}

impl SearchCursor {
    fn parse(s: &str) -> Result<SearchCursor, String> {
        let bad = || {
            format!(
                "'{s}' is not a search cursor. Copy the `next cursor:` value from the last page."
            )
        };
        let mut parts = s.split('.');
        let (Some(offset), Some(count), Some(fingerprint), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(bad());
        };
        Ok(SearchCursor {
            offset: offset.parse().map_err(|_| bad())?,
            count: count.parse().map_err(|_| bad())?,
            fingerprint: u64::from_str_radix(fingerprint, 16).map_err(|_| bad())?,
        })
    }

    pub(super) fn encode(&self) -> String {
        format!("{}.{}.{:016x}", self.offset, self.count, self.fingerprint)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchRequest {
    pub(super) text: String,
    pub(super) query: Query,
    role: Option<Role>,
    since: Option<DateTime<Utc>>,
    before: Option<DateTime<Utc>>,
    pub(super) limit: usize,
    pub(super) cursor: Option<SearchCursor>,
    pub(super) fingerprint: u64,
}

#[derive(Clone, Copy)]
enum Edge {
    Since,
    Until,
}

fn parse_edge(s: &str, edge: Edge) -> Result<DateTime<Utc>, String> {
    let s = s.trim();
    let bad = || format!("'{s}' is not a date. Use an RFC 3339 timestamp or YYYY-MM-DD.");
    if let Ok(stamp) = DateTime::parse_from_rfc3339(s) {
        let stamp = stamp.with_timezone(&Utc);
        return Ok(match edge {
            Edge::Since => stamp,
            Edge::Until => stamp + Duration::nanoseconds(1),
        });
    }
    let midnight = NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .ok()
        .and_then(|day| day.and_hms_opt(0, 0, 0))
        .map(|naive| naive.and_utc())
        .ok_or_else(bad)?;
    Ok(match edge {
        Edge::Since => midnight,
        Edge::Until => midnight + Duration::days(1),
    })
}

fn parse_role(s: &str) -> Result<Role, String> {
    match s.trim() {
        "user" => Ok(Role::User),
        "assistant" => Ok(Role::Assistant),
        "worker" => Ok(Role::Worker),
        other => Err(format!(
            "'{other}' is not a role. Use user, assistant, or worker."
        )),
    }
}

fn optional_str<'a>(args: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(format!("'{key}' must be a string.")),
    }
}

fn parse_limit(args: &Value) -> Result<usize, String> {
    let bad = || format!("limit must be a whole number from 1 to {MAX_LIMIT}.");
    match args.get("limit") {
        None | Some(Value::Null) => Ok(DEFAULT_LIMIT),
        Some(v) => v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| (1..=MAX_LIMIT).contains(n))
            .ok_or_else(bad),
    }
}

fn fingerprint(query: &Query, request: &SearchRequest) -> u64 {
    let part = |bound: Option<DateTime<Utc>>| bound.map(|b| b.to_rfc3339()).unwrap_or_default();
    let role = request.role.map_or("", Role::as_str);
    let key = [
        query.normalized(),
        role.to_string(),
        part(request.since),
        part(request.before),
        request.limit.to_string(),
    ]
    .join("\u{1f}");
    fnv1a(key.as_bytes())
}

pub fn parse_search_args(args: &Value) -> Result<SearchRequest, String> {
    let text = string_arg(args, "query")?;
    let query = Query::parse(&text)?;
    let mut request = SearchRequest {
        text,
        query,
        role: optional_str(args, "role")?.map(parse_role).transpose()?,
        since: optional_str(args, "since")?
            .map(|s| parse_edge(s, Edge::Since))
            .transpose()?,
        before: optional_str(args, "until")?
            .map(|s| parse_edge(s, Edge::Until))
            .transpose()?,
        limit: parse_limit(args)?,
        cursor: optional_str(args, "cursor")?
            .map(|s| SearchCursor::parse(s.trim()))
            .transpose()?,
        fingerprint: 0,
    };
    if request
        .since
        .zip(request.before)
        .is_some_and(|(since, before)| since >= before)
    {
        return Err("'since' is later than 'until'.".to_string());
    }
    request.fingerprint = fingerprint(&request.query, &request);
    Ok(request)
}

pub(super) fn check_cursor(request: &SearchRequest, count: usize) -> Result<(), String> {
    match request.cursor {
        Some(c) if c.count != count => Err("stale cursor; the log changed".to_string()),
        _ => Ok(()),
    }
}

pub(super) fn check_fingerprint(request: &SearchRequest) -> Result<(), String> {
    match request.cursor {
        Some(c) if c.fingerprint != request.fingerprint => {
            Err("cursor does not match this query".to_string())
        }
        _ => Ok(()),
    }
}

pub(super) fn passes(request: &SearchRequest, entry: &Entry) -> bool {
    if request.role.is_some_and(|role| role != entry.role) {
        return false;
    }
    if request.since.is_none() && request.before.is_none() {
        return true;
    }
    let Ok(at) = DateTime::parse_from_rfc3339(&entry.at) else {
        return false;
    };
    let at = at.with_timezone(&Utc);
    request.since.is_none_or(|since| at >= since) && request.before.is_none_or(|before| at < before)
}
