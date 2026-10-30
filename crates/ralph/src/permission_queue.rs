use crate::acp_worker::{push_event, Events};
use crate::worker_mcp::Inbox;
use ralph_core::worker::{
    match_option, no_option_message, pending_position, permission_alert_report, BlockedContent,
    PermissionOption, WorkerEvent,
};
use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, PoisonError};

struct Pending<R> {
    reply: R,
    content: BlockedContent,
}

pub(crate) struct PermissionQueue<R> {
    events: Events,
    worker_id: String,
    inbox: Inbox,
    queue: Mutex<VecDeque<Pending<R>>>,
}

impl<R> PermissionQueue<R> {
    pub(crate) fn new(worker_id: String, inbox: Inbox, events: Events) -> Self {
        Self {
            events,
            worker_id,
            inbox,
            queue: Mutex::new(VecDeque::new()),
        }
    }

    fn queue(&self) -> MutexGuard<'_, VecDeque<Pending<R>>> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn publish(&self, queue: &VecDeque<Pending<R>>) {
        let event = match queue.front() {
            Some(front) => WorkerEvent::PermissionAsked(BlockedContent {
                pending: queue
                    .iter()
                    .map(|pending| pending.content.request_id.clone())
                    .collect(),
                ..front.content.clone()
            }),
            None => WorkerEvent::PermissionSettled,
        };
        push_event(&self.events, event);
    }

    pub(crate) fn enqueue(&self, reply: R, content: BlockedContent) {
        let at = chrono::Utc::now().to_rfc3339();
        let report = permission_alert_report(&self.worker_id, &at, &content);
        let mut queue = self.queue();
        queue.push_back(Pending { reply, content });
        self.publish(&queue);
        self.inbox.post(&report, 0).ok();
    }

    pub(crate) fn take_answer(
        &self,
        request_id: &str,
        text: &str,
    ) -> Result<(R, PermissionOption), String> {
        let mut queue = self.queue();
        let ids = queue
            .iter()
            .map(|pending| pending.content.request_id.as_str());
        let index = pending_position(ids, request_id)?;
        let options = &queue[index].content.options;
        let option = match_option(options, text)
            .ok_or_else(|| no_option_message(options))?
            .clone();
        let pending = queue
            .remove(index)
            .ok_or_else(|| format!("stale request {request_id}"))?;
        self.publish(&queue);
        Ok((pending.reply, option))
    }

    pub(crate) fn settle(&self, matches: impl Fn(&R) -> bool) {
        let mut queue = self.queue();
        let before = queue.len();
        queue.retain(|pending| !matches(&pending.reply));
        if queue.len() != before {
            self.publish(&queue);
        }
    }

    pub(crate) fn drain(&self) -> Vec<R> {
        let drained = self
            .queue()
            .drain(..)
            .map(|pending| pending.reply)
            .collect();
        push_event(&self.events, WorkerEvent::PermissionSettled);
        drained
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn content(request: &str, ids: &[&str]) -> BlockedContent {
        BlockedContent {
            request_id: request.into(),
            tool: "shell".into(),
            summary: "ls".into(),
            options: ids
                .iter()
                .map(|id| PermissionOption {
                    option_id: (*id).into(),
                    name: (*id).into(),
                    kind: "allow_once".into(),
                })
                .collect(),
            pending: Vec::new(),
        }
    }

    fn queue(dir: &TempDir) -> (PermissionQueue<u32>, Events) {
        let events = Events::default();
        let inbox = Inbox::new(dir.path().join("inbox"));
        let queue = PermissionQueue::new("w".into(), inbox, Arc::clone(&events));
        (queue, events)
    }

    fn drained(events: &Events) -> Vec<WorkerEvent> {
        std::mem::take(&mut *events.lock().unwrap())
    }

    #[test]
    fn an_answer_takes_the_request_it_names_and_refuses_an_unpublished_option() {
        let dir = TempDir::new().unwrap();
        let (queue, events) = queue(&dir);
        queue.enqueue(1, content("a", &["accept", "cancel"]));
        queue.enqueue(2, content("b", &["decline"]));
        assert!(
            matches!(drained(&events).last(), Some(WorkerEvent::PermissionAsked(c)) if c.pending == ["a", "b"])
        );
        assert!(queue.take_answer("b", "accept").is_err());
        let (reply, option) = queue.take_answer(" b ", " decline\n").unwrap();
        assert_eq!((reply, option.option_id.as_str()), (2, "decline"));
        let events = drained(&events);
        assert!(
            matches!(events.last(), Some(WorkerEvent::PermissionAsked(c)) if c.request_id == "a" && c.pending == ["a"])
        );
    }

    #[test]
    fn a_settled_answered_or_unknown_request_is_rejected_as_stale() {
        let dir = TempDir::new().unwrap();
        let (queue, events) = queue(&dir);
        queue.enqueue(7, content("a", &["accept"]));
        queue.enqueue(8, content("b", &["accept"]));
        queue.settle(|reply| *reply == 7);
        assert_eq!(
            queue.take_answer("a", "accept").unwrap_err(),
            "stale request a; pending: b"
        );
        assert_eq!(
            queue.take_answer("zz", "accept").unwrap_err(),
            "stale request zz; pending: b"
        );
        assert_eq!(queue.take_answer("b", "accept").unwrap().0, 8);
        assert_eq!(
            queue.take_answer("b", "accept").unwrap_err(),
            "stale request b; pending: none"
        );
        assert_eq!(
            drained(&events).last(),
            Some(&WorkerEvent::PermissionSettled)
        );
    }

    #[test]
    fn draining_drops_every_pending_request_and_settles() {
        let dir = TempDir::new().unwrap();
        let (queue, events) = queue(&dir);
        queue.enqueue(1, content("a", &["accept"]));
        queue.enqueue(2, content("b", &["accept"]));
        assert_eq!(queue.drain(), vec![1, 2]);
        assert!(queue.take_answer("a", "accept").is_err());
        assert_eq!(
            drained(&events).last(),
            Some(&WorkerEvent::PermissionSettled)
        );
    }
}
