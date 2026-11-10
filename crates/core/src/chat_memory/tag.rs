//! Pure side of entry tagging: the prompt, batches, reply parsing and the
//! `tags.jsonl` rows. The shell appends the rows and runs the model.

use super::{Entry, Role};
use crate::retry::{classify, FailureInput, FailureKind};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const TAG_PROMPT: &str = include_str!("tag_prompt.txt");
pub const PROMPT_VERSION: &str = "v2";
pub const TAG_MODEL: &str = "claude-haiku-5-5";
pub const TAGS_FILE: &str = "tags.jsonl";
/// Most entries one pass tags: a backstop against a huge backlog.
pub const MAX_PER_PASS: usize = 100;
pub const BATCH_LADDER: [usize; 3] = [20, 5, 1];
const TARGET_CHARS: usize = 1200;
const CONTEXT_CHARS: usize = 350;
const CONTEXT_ENTRIES: usize = 3;
const ROW_VERSION: u32 = 1;
const SKIP_AFTER_FAILURES: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flags {
    pub rule: bool,
    pub decision: bool,
    pub approval: bool,
}

pub fn restrict_decision(role: Role, flags: Flags) -> Flags {
    Flags {
        decision: flags.decision && role == Role::User,
        ..flags
    }
}

/// One of the three labels a search can filter on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Label {
    Rule,
    Decision,
    Approval,
}

impl Label {
    pub fn parse(s: &str) -> Result<Label, String> {
        match s.trim() {
            "rule" => Ok(Label::Rule),
            "decision" => Ok(Label::Decision),
            "approval" => Ok(Label::Approval),
            other => Err(format!(
                "'{other}' is not a tag. Use rule, decision or approval."
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Label::Rule => "rule",
            Label::Decision => "decision",
            Label::Approval => "approval",
        }
    }

    fn set_in(self, flags: Flags) -> bool {
        match self {
            Label::Rule => flags.rule,
            Label::Decision => flags.decision,
            Label::Approval => flags.approval,
        }
    }
}

/// Ids of the entries whose latest tag row carries `label`. The search joins
/// them to entries and drops a `decision` hit on a non-user entry (`passes`).
pub fn ids_with(state: &TagState, label: Label) -> BTreeSet<usize> {
    state
        .tags
        .iter()
        .filter(|(_, flags)| label.set_in(**flags))
        .map(|(id, _)| *id)
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowKind {
    Tagged(Flags),
    Failed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagRow {
    pub n: usize,
    pub kind: RowKind,
    pub model: String,
    pub prompt: String,
    pub at: String,
}

/// What `tags.jsonl` says: where tagging starts, the tags, and how many failed
/// passes each entry has had.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TagState {
    pub start: Option<usize>,
    pub tags: BTreeMap<usize, Flags>,
    pub failures: BTreeMap<usize, u32>,
    pub skipped: BTreeSet<usize>,
}

pub fn encode_start(start: usize, at: &str) -> Result<String, String> {
    line(&json!({"v": ROW_VERSION, "start": start, "at": at}))
}

pub fn encode_row(row: &TagRow) -> Result<String, String> {
    let mut value = json!({"v": ROW_VERSION, "n": row.n});
    if let (Some(map), RowKind::Tagged(flags)) = (value.as_object_mut(), &row.kind) {
        map.insert("rule".into(), flags.rule.into());
        map.insert("decision".into(), flags.decision.into());
        map.insert("approval".into(), flags.approval.into());
    }
    if let Some(map) = value.as_object_mut() {
        match row.kind {
            RowKind::Failed => map.insert("failed".into(), true.into()),
            RowKind::Skipped => map.insert("skipped".into(), true.into()),
            RowKind::Tagged(_) => None,
        };
        map.insert("model".into(), row.model.clone().into());
        map.insert("prompt".into(), row.prompt.clone().into());
        map.insert("at".into(), row.at.clone().into());
    }
    line(&value)
}

fn line(value: &Value) -> Result<String, String> {
    let mut text = serde_json::to_string(value).map_err(|e| e.to_string())?;
    text.push('\n');
    Ok(text)
}

/// Reads every whole line; torn or unknown lines are ignored and a later tag
/// for the same entry replaces an earlier one.
pub fn decode_rows(data: &[u8]) -> TagState {
    let mut state = TagState::default();
    let text = String::from_utf8_lossy(data);
    for raw in text.split_inclusive('\n').filter(|l| l.ends_with('\n')) {
        let Ok(value) = serde_json::from_str::<Value>(raw) else {
            continue;
        };
        if let Some(start) = value.get("start").and_then(Value::as_u64) {
            state.start = Some(start as usize);
            continue;
        }
        let Some(n) = value.get("n").and_then(Value::as_u64).map(|n| n as usize) else {
            continue;
        };
        if value.get("skipped").and_then(Value::as_bool) == Some(true) {
            state.skipped.insert(n);
        } else if value.get("failed").and_then(Value::as_bool) == Some(true) {
            *state.failures.entry(n).or_insert(0) += 1;
        } else if let Some(flags) = flags_of(&value) {
            state.tags.insert(n, flags);
        }
    }
    state
}

fn flags_of(value: &Value) -> Option<Flags> {
    let flag = |key: &str| value.get(key).and_then(Value::as_bool);
    Some(Flags {
        rule: flag("rule")?,
        decision: flag("decision").unwrap_or(false),
        approval: flag("approval")?,
    })
}

/// Entry ids from the start row on that have no tag and no skip. Corrupt
/// entries have no text to tag and are left out. Without a start row nothing
/// is due: the shell writes the start row first.
pub fn untagged(entries: &[Entry], state: &TagState) -> Vec<usize> {
    let Some(start) = state.start else {
        return Vec::new();
    };
    (start..entries.len())
        .filter(|id| {
            !entries[*id].is_corrupt()
                && !state.tags.contains_key(id)
                && !state.skipped.contains(id)
        })
        .collect()
}

fn cut(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagPlan {
    Run,
    SkipAhead,
    Skip,
}

/// What a tag turn does, from the result of loading the `tag_entries` setting
/// and whether the store's `config.toml` failed to load (the load then falls
/// back to the global config, whose value may differ from the store's).
/// A setting that cannot be read is no consent to paid calls: the turn does
/// nothing, and the store is not touched.
pub fn tag_plan<E>(tag_entries: Result<bool, E>, store_layer_failed: bool) -> TagPlan {
    match tag_entries {
        _ if store_layer_failed => TagPlan::Skip,
        Ok(true) => TagPlan::Run,
        Ok(false) => TagPlan::SkipAhead,
        Err(_) => TagPlan::Skip,
    }
}

/// Entries whose text must stay out of batch context: those with a stored
/// `failed` row and no tag, or a `skipped` row: they failed alone in an earlier
/// pass. A later tag clears a `failed` row.
pub fn context_exclusions(state: &TagState) -> BTreeSet<usize> {
    state
        .failures
        .keys()
        .filter(|n| !state.tags.contains_key(n))
        .chain(state.skipped.iter())
        .copied()
        .collect()
}

/// The batch text. Entries in `exclude` (ones that failed alone, see `context_exclusions`)
/// never ride along as context, or their text would fail the calls of the
/// entries after them.
pub fn build_batch(entries: &[Entry], ids: &[usize], exclude: &BTreeSet<usize>) -> String {
    let mut out = String::new();
    for &id in ids {
        let Some(target) = entries.get(id) else {
            continue;
        };
        out.push_str(&format!("### ENTRY {id} ({})\n", target.role.as_str()));
        let first = id.saturating_sub(CONTEXT_ENTRIES);
        for (ctx, entry) in entries.iter().enumerate().take(id).skip(first) {
            if exclude.contains(&ctx) {
                continue;
            }
            out.push_str(&format!(
                "  [context #{ctx} {}] {}\n",
                entry.role.as_str(),
                cut(&entry.text, CONTEXT_CHARS)
            ));
        }
        out.push_str(&cut(&target.text, TARGET_CHARS));
        out.push_str("\n\n");
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub tagged: Vec<(usize, Flags)>,
    pub missing: Vec<usize>,
}

/// The text to parse from an exit-0 call. An empty reply is an empty string,
/// so its entries count as missing and the ladder handles them.
fn reply_text(response: Option<&str>) -> String {
    response.map(str::trim).unwrap_or_default().to_string()
}

/// Whether a pass that last read `seen` entries must take `TAG` again: the
/// log grew after that read and before the lock was released.
pub fn retake(len_now: usize, seen: usize) -> bool {
    len_now > seen
}

/// Classifies a failed tag call. `claude -p` prints an API error such as
/// "API Error: 529 ..." as its reply, with an empty stderr, so on a non-zero
/// exit the reply counts, but only through its lines that start with
/// `API Error`: the shared classifier reads bare codes such as "503" anywhere,
/// and a tag reply is full of entry numbers. An exit-0 reply never counts.
pub fn tag_failure_kind(
    timed_out: bool,
    exit_code: i32,
    stderr: &str,
    response: &str,
) -> FailureKind {
    let errors: Vec<&str> = response
        .lines()
        .map(str::trim)
        .filter(|line| line.to_lowercase().starts_with("api error"))
        .collect();
    let text = format!("{stderr}\n{}", errors.join("\n"));
    classify(&FailureInput {
        timed_out,
        exit_code,
        text: &text,
    })
}

/// What a finished tag call gives the pass: the reply text, and whether the
/// process exited 0. A non-zero exit has no text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallReply {
    pub text: String,
    pub exited_ok: bool,
}

/// How many calls in a row may exit non-zero before the pass reads the
/// failure as global and stops. A good call only resets the count.
const MAX_BAD_STREAK: u32 = 4;

/// What a pass has seen of its own calls, and which entries that isolated.
/// An entry that failed alone waits in `pending` until a later call exits 0;
/// then it is `confirmed` and the shell may write its `failed` row.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PassHealth {
    bad_streak: u32,
    pending: Vec<usize>,
    confirmed: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Go,
    Stop,
}

/// Folds one finished call into the pass health. A global fault (a bad login,
/// credit, a flag) makes every call fail, which text cannot tell from one bad
/// entry. So the pass stops at `MAX_BAD_STREAK` non-zero exits in a row, even
/// after earlier good calls. A good call resets the count and confirms the
/// entries isolated before it; a stop drops the pending ones.
pub fn after_call(health: PassHealth, exited_ok: bool) -> (PassHealth, Verdict) {
    let mut health = health;
    if exited_ok {
        health.bad_streak = 0;
        let waiting = std::mem::take(&mut health.pending);
        health.confirmed.extend(waiting);
        return (health, Verdict::Go);
    }
    health.bad_streak += 1;
    if health.bad_streak >= MAX_BAD_STREAK {
        health.pending.clear();
        return (health, Verdict::Stop);
    }
    (health, Verdict::Go)
}

/// Records that `id` failed alone, on a call that exited 0 or not. After an
/// exit-0 call it is confirmed at once: the model answered and left it out.
/// After a non-zero exit it waits for a later call to exit 0.
pub fn isolated(health: PassHealth, id: usize, exited_ok: bool) -> PassHealth {
    let mut health = health;
    if exited_ok {
        health.confirmed.push(id);
    } else {
        health.pending.push(id);
    }
    health
}

/// The entries confirmed so far, and the health without them.
pub fn take_confirmed(health: PassHealth) -> (PassHealth, Vec<usize>) {
    let mut health = health;
    let confirmed = std::mem::take(&mut health.confirmed);
    (health, confirmed)
}

/// The reply of a finished call, or an error when the call did not finish. An
/// outage that outlasted the backoff and a process told to stop end the pass
/// with no rows. A non-zero exit gives no text, so the ladder isolates the
/// entries; `after_call` and `isolated` keep a global fault from marking them.
pub fn tag_reply(
    failure: Option<FailureKind>,
    terminated: bool,
    exit_code: i32,
    response: Option<&str>,
) -> Result<CallReply, String> {
    if terminated {
        return Err("tagging was interrupted".to_string());
    }
    if failure == Some(FailureKind::Outage) {
        return Err("the model is out; tagging stops until the next reply".to_string());
    }
    let exited_ok = exit_code == 0;
    let text = if exited_ok {
        reply_text(response)
    } else {
        String::new()
    };
    Ok(CallReply { text, exited_ok })
}

/// Keeps each asked entry's first complete row; anything else is missing.
pub fn parse_reply(reply: &str, asked: &[usize]) -> Parsed {
    let rows: Vec<Value> = reply
        .find('[')
        .zip(reply.rfind(']'))
        .and_then(|(lo, hi)| reply.get(lo..=hi))
        .and_then(|array| serde_json::from_str(array).ok())
        .unwrap_or_default();
    let mut tagged: Vec<(usize, Flags)> = Vec::new();
    for value in &rows {
        let n = value.get("n").and_then(Value::as_u64).map(|n| n as usize);
        let flags =
            flags_of(value).filter(|_| value.get("decision").is_some_and(Value::is_boolean));
        if let (Some(n), Some(flags)) = (n, flags) {
            if asked.contains(&n) && !tagged.iter().any(|(seen, _)| *seen == n) {
                tagged.push((n, flags));
            }
        }
    }
    let missing = asked
        .iter()
        .copied()
        .filter(|id| !tagged.iter().any(|(n, _)| n == id))
        .collect();
    Parsed { tagged, missing }
}

pub fn next_size(size: usize) -> Option<usize> {
    let at = BATCH_LADDER.iter().position(|&rung| rung == size)?;
    BATCH_LADDER.get(at + 1).copied()
}

pub fn batches(ids: &[usize], size: usize) -> Vec<Vec<usize>> {
    ids.chunks(size.max(1)).map(<[usize]>::to_vec).collect()
}

/// The row for an entry that failed even alone: `Failed` the first time,
/// `Skipped` once it has failed on `SKIP_AFTER_FAILURES` passes.
pub fn failure_kind(earlier_failures: u32) -> RowKind {
    if earlier_failures + 1 >= SKIP_AFTER_FAILURES {
        RowKind::Skipped
    } else {
        RowKind::Failed
    }
}

#[cfg(test)]
#[path = "tag_tests.rs"]
mod tests;
