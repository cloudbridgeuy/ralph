use super::{Item, Kind, STAGE_EXPIRY_LOG_DISTANCE};
use crate::chat_status::{ScheduleState, Status, WatchState, WorkerState};
use crate::chat_thread::ThreadName;
use crate::worker::Attention;
use std::fmt;

const LOG_ROWS_PER_TURN: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageItemId(String);

impl fmt::Display for StageItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketRef(String);

impl fmt::Display for TicketRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StageInput<'a> {
    pub items: &'a [Item],
    pub status: &'a Status,
    pub thread: Option<&'a ThreadName>,
    pub log_len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemRow {
    pub id: StageItemId,
    pub reference: Option<TicketRef>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Need {
    Decision(ItemRow),
    Worker(WorkerState),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenKind {
    Owed,
    Followup,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenRow {
    pub kind: OpenKind,
    pub row: ItemRow,
    pub turns_left: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fact {
    Workers(Vec<WorkerState>),
    Watches(Vec<WatchState>),
    Schedule(Vec<ScheduleState>),
    Inbox(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageView {
    pub thread: Option<ThreadName>,
    pub needs: Vec<Need>,
    pub open: Vec<OpenRow>,
    pub running: Vec<Fact>,
}

pub fn turns_left(item: &Item, log_len: usize) -> usize {
    STAGE_EXPIRY_LOG_DISTANCE
        .saturating_sub(log_len.saturating_sub(item.turn))
        .div_ceil(LOG_ROWS_PER_TURN)
}

fn item_row(item: &Item) -> ItemRow {
    ItemRow {
        id: StageItemId(item.id.clone()),
        reference: item.reference.clone().map(TicketRef),
        text: item.text.clone(),
    }
}

fn open_row(item: &Item, log_len: usize) -> Option<OpenRow> {
    let kind = match item.kind {
        Kind::Decision => return None,
        Kind::Owed => OpenKind::Owed,
        Kind::Followup => OpenKind::Followup,
    };
    Some(OpenRow {
        kind,
        row: item_row(item),
        turns_left: turns_left(item, log_len),
    })
}

pub fn stage_view(input: &StageInput) -> Option<StageView> {
    let status = input.status;
    let (needing, other): (Vec<WorkerState>, Vec<WorkerState>) = status
        .workers
        .iter()
        .cloned()
        .partition(|worker| worker.state.attention() == Attention::NeedsUser);
    let needs: Vec<Need> = input
        .items
        .iter()
        .filter(|item| item.kind == Kind::Decision)
        .map(|item| Need::Decision(item_row(item)))
        .chain(needing.into_iter().map(Need::Worker))
        .collect();
    let open: Vec<OpenRow> = input
        .items
        .iter()
        .filter_map(|item| open_row(item, input.log_len))
        .collect();
    let running: Vec<Fact> = [
        (!other.is_empty()).then_some(Fact::Workers(other)),
        (!status.watches.is_empty()).then(|| Fact::Watches(status.watches.clone())),
        (!status.schedule.is_empty()).then(|| Fact::Schedule(status.schedule.clone())),
        (status.unread > 0).then_some(Fact::Inbox(status.unread)),
    ]
    .into_iter()
    .flatten()
    .collect();
    let shown =
        input.thread.is_some() || !needs.is_empty() || !open.is_empty() || !running.is_empty();
    shown.then(|| StageView {
        thread: input.thread.cloned(),
        needs,
        open,
        running,
    })
}

#[cfg(test)]
mod tests;
