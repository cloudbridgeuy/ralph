use crate::chat_spec::Keyword;
use crate::chat_status::Status;
use crate::chat_suggestion::Suggestion;
use crate::chat_thread::ThreadName;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const STAGE_EXPIRY_LOG_DISTANCE: usize = 60;
pub const STAGE_REPEAT_TURNS: usize = 5;
pub const STAGE_ADD: &str = "stage_add";
pub const STAGE_DONE: &str = "stage_done";
pub const STAGE_EMPTY_LINE: &str = "Stage: empty";
pub const STAGE_USAGE: &str = "Usage:\n/stage\n/stage add <decision|owed|followup> <text>\n/stage done <id>\n/stage edit <id> <text>";

const NEEDS_USER_STATES: [&str; 2] = ["question", "permission"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Decision,
    Owed,
    Followup,
}

impl Keyword for Kind {
    const ALL: &'static [Self] = &[Kind::Decision, Kind::Owed, Kind::Followup];

    fn word(self) -> &'static str {
        match self {
            Kind::Decision => "decision",
            Kind::Owed => "owed",
            Kind::Followup => "followup",
        }
    }
}

impl Kind {
    fn expires(self) -> bool {
        self != Kind::Decision
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub kind: Kind,
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    pub text: String,
    pub created: String,
    pub turn: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stage {
    pub items: Vec<Item>,
    pub last: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Actor {
    Model,
    User,
    Expiry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Op {
    Add,
    Done,
    Edit,
    Expire,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditRow {
    pub ts: String,
    pub actor: Actor,
    pub op: Op,
    pub item: Item,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl AuditRow {
    pub fn new(ts: &str, actor: Actor, op: Op, item: &Item) -> Self {
        Self {
            ts: ts.to_string(),
            actor,
            op,
            item: item.clone(),
            note: None,
        }
    }

    pub fn with_note(self, note: Option<String>) -> Self {
        Self { note, ..self }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewItem {
    pub kind: Kind,
    pub reference: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Closing {
    pub id: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageMark {
    pub text: String,
    pub turns_since: usize,
}

impl StageMark {
    pub fn sent(text: &str) -> Self {
        Self {
            text: text.to_string(),
            turns_since: 0,
        }
    }

    pub fn skipped(&self) -> Self {
        Self {
            text: self.text.clone(),
            turns_since: self.turns_since + 1,
        }
    }
}

pub fn next_mark(mark: Option<&StageMark>, launched: bool, due: bool, rendered: &str) -> StageMark {
    match mark {
        Some(held) if !launched && !due => held.skipped(),
        _ => StageMark::sent(rendered),
    }
}

pub fn expire(stage: Stage, log_len: usize) -> (Stage, Vec<Item>) {
    let (gone, items) = stage.items.into_iter().partition(|item| {
        item.kind.expires() && log_len.saturating_sub(item.turn) >= STAGE_EXPIRY_LOG_DISTANCE
    });
    (
        Stage {
            items,
            last: stage.last,
        },
        gone,
    )
}

pub fn thread_line(name: &ThreadName) -> String {
    format!("thread {name}: unresolved, end with /thread merge or drop")
}

pub fn render(items: &[Item], status: &Status, thread: Option<&ThreadName>) -> String {
    let item_lines = items.iter().map(|item| match &item.reference {
        Some(reference) => format!(
            "{} {} {reference}: {}",
            item.id,
            item.kind.word(),
            item.text
        ),
        None => format!("{} {} {}", item.id, item.kind.word(), item.text),
    });
    let (needing, other): (Vec<_>, Vec<_>) = status
        .workers
        .iter()
        .partition(|worker| NEEDS_USER_STATES.contains(&worker.state.as_str()));
    let facts = [
        line(
            "needs you",
            needing.iter().map(|w| format!("{} {}", w.id, w.state)),
        ),
        line("workers", other.iter().map(|w| w.id.clone())),
        line("watches", status.watches.iter().map(|w| w.id.clone())),
        line(
            "schedule",
            status
                .schedule
                .iter()
                .map(|s| format!("{} {}", s.id, s.time)),
        ),
        (status.unread > 0).then(|| format!("inbox: {} unread", status.unread)),
    ];
    let lines: Vec<String> = thread
        .map(thread_line)
        .into_iter()
        .chain(item_lines)
        .chain(facts.into_iter().flatten())
        .collect();
    if lines.is_empty() {
        return String::new();
    }
    format!("Stage:\n{}", lines.join("\n"))
}

fn line(label: &str, entries: impl Iterator<Item = String>) -> Option<String> {
    let entries: Vec<String> = entries.collect();
    (!entries.is_empty()).then(|| format!("{label}: {}", entries.join(", ")))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub label: String,
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gap {
    pub unnamed: Vec<String>,
}

impl Gap {
    pub fn notice(&self) -> String {
        format!(
            "Next step skips pending decision: {}",
            self.unnamed.join(", ")
        )
    }
}

pub fn suggestion_after(suggestion: Option<String>, gap: Option<&Gap>) -> Option<Suggestion> {
    suggestion.map(|text| match gap {
        Some(_) => Suggestion::PrefillOnly(text),
        None => Suggestion::Open(text),
    })
}

pub fn pending_decisions(items: &[Item], status: &Status) -> Vec<Pending> {
    let decisions = items
        .iter()
        .filter(|item| item.kind == Kind::Decision)
        .map(|item| match &item.reference {
            Some(reference) => Pending {
                label: format!("{} ({reference})", item.id),
                keys: vec![item.id.clone(), reference.clone()],
            },
            None => Pending {
                label: item.id.clone(),
                keys: vec![item.id.clone()],
            },
        });
    let workers = status
        .workers
        .iter()
        .filter(|worker| NEEDS_USER_STATES.contains(&worker.state.as_str()))
        .map(|worker| Pending {
            label: format!("{} {}", worker.id, worker.state),
            keys: vec![worker.id.clone()],
        });
    decisions.chain(workers).collect()
}

fn names(text: &str, key: &str) -> bool {
    let token = |c: char| c.is_alphanumeric() || c == '-' || c == '_';
    text.match_indices(key).any(|(start, found)| {
        let before = text[..start].chars().next_back();
        let after = text[start + found.len()..].chars().next();
        !before.is_some_and(token) && !after.is_some_and(token)
    })
}

pub fn next_step_gap(next: Option<&str>, pending: &[Pending]) -> Option<Gap> {
    let step = next.filter(|step| !step.starts_with("/hold")).unwrap_or("");
    let unnamed: Vec<String> = pending
        .iter()
        .filter(|entry| !entry.keys.iter().any(|key| names(step, key)))
        .map(|entry| entry.label.clone())
        .collect();
    (!pending.is_empty() && unnamed.len() == pending.len()).then_some(Gap { unnamed })
}

pub fn stage_due(mark: Option<&StageMark>, rendered: &str, thread: Option<&ThreadName>) -> bool {
    if thread.is_some() {
        return true;
    }
    match mark {
        None => !rendered.is_empty(),
        Some(mark) => {
            mark.text != rendered
                || (!rendered.is_empty() && mark.turns_since >= STAGE_REPEAT_TURNS)
        }
    }
}

pub fn stage_text(rendered: &str) -> &str {
    if rendered.is_empty() {
        STAGE_EMPTY_LINE
    } else {
        rendered
    }
}

pub fn add(stage: Stage, new: NewItem, created: &str, turn: usize) -> (Stage, Item) {
    let last = stage.last + 1;
    let item = Item {
        id: format!("s{last}"),
        kind: new.kind,
        reference: new.reference,
        text: new.text,
        created: created.to_string(),
        turn,
    };
    let mut items = stage.items;
    items.push(item.clone());
    (Stage { items, last }, item)
}

pub fn close(stage: Stage, id: &str) -> Result<(Stage, Item), String> {
    let position = stage
        .items
        .iter()
        .position(|item| item.id == id)
        .ok_or_else(|| format!("No open stage item {id}."))?;
    let mut items = stage.items;
    let item = items.remove(position);
    Ok((
        Stage {
            items,
            last: stage.last,
        },
        item,
    ))
}

pub fn edit(stage: Stage, id: &str, text: &str) -> Result<(Stage, Item), String> {
    let mut items = stage.items;
    let item = items
        .iter_mut()
        .find(|item| item.id == id)
        .ok_or_else(|| format!("No open stage item {id}."))?;
    item.text = text.to_string();
    let edited = item.clone();
    Ok((
        Stage {
            items,
            last: stage.last,
        },
        edited,
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageVerb {
    Add,
    Done,
    Edit,
}

impl Keyword for StageVerb {
    const ALL: &'static [Self] = &[StageVerb::Add, StageVerb::Done, StageVerb::Edit];

    fn word(self) -> &'static str {
        match self {
            StageVerb::Add => "add",
            StageVerb::Done => "done",
            StageVerb::Edit => "edit",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageCommand {
    Show,
    Add(NewItem),
    Done(String),
    Edit { id: String, text: String },
    Usage,
}

pub fn parse_stage_args(args: &str) -> StageCommand {
    let (verb, rest) = split_word(args);
    let parsed = match (verb, StageVerb::parse(verb)) {
        ("", _) => Ok(StageCommand::Show),
        (_, Some(StageVerb::Add)) => {
            let (kind, text) = split_word(rest);
            parse_add(&json!({"kind": kind, "text": text})).map(StageCommand::Add)
        }
        (_, Some(StageVerb::Done)) => one_word(rest).map(StageCommand::Done),
        (_, Some(StageVerb::Edit)) => {
            let (id, text) = split_word(rest);
            one_line(Some(text), "text").map(|text| StageCommand::Edit {
                id: id.to_string(),
                text,
            })
        }
        (_, None) => Err(String::new()),
    };
    parsed.unwrap_or(StageCommand::Usage)
}

fn split_word(text: &str) -> (&str, &str) {
    let text = text.trim();
    text.split_once(char::is_whitespace)
        .map_or((text, ""), |(word, rest)| (word, rest.trim_start()))
}

fn one_word(text: &str) -> Result<String, String> {
    let text = one_line(Some(text), "id")?;
    if text.contains(char::is_whitespace) {
        Err("id must be one word.".to_string())
    } else {
        Ok(text)
    }
}

pub fn stage_add_tool() -> Value {
    json!({
        "name": STAGE_ADD,
        "description": "Add an item to the stage when you leave something open: a decision waiting on the user, a question owed to a worker, or a follow-up you will do. Close it with stage_done when it closes. When a decision is open, name its sN or ref in your Next step line. Owed and followup items expire after 30 turns; decisions stay until closed.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "kind": {"type": "string", "enum": ["decision", "owed", "followup"]},
                "text": {"type": "string", "description": "One line of plain text."},
                "ref": {"type": "string", "description": "Optional: a ticket, worker or branch the item is about."}
            },
            "required": ["kind", "text"]
        }
    })
}

pub fn stage_done_tool() -> Value {
    json!({
        "name": STAGE_DONE,
        "description": "Close a stage item by its id (s4) when what it tracked is settled.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "The item id, such as s4."},
                "note": {"type": "string", "description": "Optional: how it closed, one line."}
            },
            "required": ["id"]
        }
    })
}

pub fn parse_add(args: &Value) -> Result<NewItem, String> {
    let kind = args["kind"]
        .as_str()
        .and_then(Kind::parse)
        .ok_or_else(|| "kind must be decision, owed or followup.".to_string())?;
    let text = one_line(args["text"].as_str(), "text")?;
    let reference = optional_line(args, "ref")?;
    Ok(NewItem {
        kind,
        reference,
        text,
    })
}

pub fn parse_done(args: &Value) -> Result<Closing, String> {
    Ok(Closing {
        id: one_line(args["id"].as_str(), "id")?,
        note: optional_line(args, "note")?,
    })
}

fn optional_line(args: &Value, name: &str) -> Result<Option<String>, String> {
    args.get(name)
        .filter(|value| !value.is_null())
        .map(|value| one_line(value.as_str(), name))
        .transpose()
}

fn one_line(raw: Option<&str>, name: &str) -> Result<String, String> {
    let text = raw.unwrap_or_default().trim();
    if text.is_empty() {
        Err(format!("{name} is missing or empty."))
    } else if text.chars().any(char::is_control) {
        Err(format!("{name} must be one line of plain text."))
    } else {
        Ok(text.to_string())
    }
}

#[cfg(test)]
mod tests;
