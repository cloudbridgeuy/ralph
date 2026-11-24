use crate::chat_status::Status;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const STAGE_EXPIRY_LOG_DISTANCE: usize = 60;
pub const STAGE_REPEAT_TURNS: usize = 5;
pub const STAGE_EMPTY_LINE: &str = "Stage: empty";

const NEEDS_USER_STATES: [&str; 3] = ["question", "blocked", "permission"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Decision,
    Owed,
    Followup,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Decision => "decision",
            Kind::Owed => "owed",
            Kind::Followup => "followup",
        }
    }

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
    Expire,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditRow {
    pub ts: String,
    pub actor: Actor,
    pub op: Op,
    pub item: Item,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewItem {
    pub kind: Kind,
    pub reference: Option<String>,
    pub text: String,
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

pub fn render(items: &[Item], status: &Status) -> String {
    let item_lines = items.iter().map(|item| match &item.reference {
        Some(reference) => format!(
            "{} {} {reference}: {}",
            item.id,
            item.kind.name(),
            item.text
        ),
        None => format!("{} {} {}", item.id, item.kind.name(), item.text),
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
    let lines: Vec<String> = item_lines.chain(facts.into_iter().flatten()).collect();
    if lines.is_empty() {
        return String::new();
    }
    format!("Stage:\n{}", lines.join("\n"))
}

fn line(label: &str, entries: impl Iterator<Item = String>) -> Option<String> {
    let entries: Vec<String> = entries.collect();
    (!entries.is_empty()).then(|| format!("{label}: {}", entries.join(", ")))
}

pub fn stage_due(mark: Option<&StageMark>, rendered: &str) -> bool {
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

pub fn parse_add(args: &Value) -> Result<NewItem, String> {
    let kind = match args["kind"].as_str() {
        Some("decision") => Kind::Decision,
        Some("owed") => Kind::Owed,
        Some("followup") => Kind::Followup,
        _ => return Err("kind must be decision, owed or followup.".to_string()),
    };
    let text = one_line(args["text"].as_str(), "text")?;
    let reference = match args.get("ref").filter(|value| !value.is_null()) {
        Some(value) => Some(one_line(value.as_str(), "ref")?),
        None => None,
    };
    Ok(NewItem {
        kind,
        reference,
        text,
    })
}

pub fn parse_done(args: &Value) -> Result<String, String> {
    one_line(args["id"].as_str(), "id")
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
