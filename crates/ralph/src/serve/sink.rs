use super::clients::{write_frame, Writer};
use ralph_core::chat_process::SessionKey;
use ralph_core::turn_event::{TurnEvent, TurnSink};
use ralph_core::worker::Report;
use ralph_server::chat::host::HostEvent;
use ralph_server::chat::serve_proto::ServerMsg;
use std::sync::Mutex;

pub(super) fn inbox_sink(
    key: SessionKey,
    deliver: impl Fn(&SessionKey, Vec<Report>, usize),
) -> impl Fn(HostEvent) {
    move |event| {
        if let HostEvent::Fresh { fresh, unread } = event {
            deliver(&key, fresh, unread);
        }
    }
}

#[derive(Default)]
pub(super) struct SocketSink {
    bound: Mutex<Option<(u64, Writer)>>,
}

impl SocketSink {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<(u64, Writer)>> {
        self.bound
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(super) fn bind(&self, id: u64, writer: Writer) {
        *self.lock() = Some((id, writer));
    }

    pub(super) fn unbind(&self) {
        self.lock().take();
    }
}

impl TurnSink for SocketSink {
    fn emit(&self, event: TurnEvent) {
        let bound = self.lock().clone();
        if let Some((id, writer)) = bound {
            let _ = write_frame(&writer, id, &ServerMsg::Turn { event });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::chat_thread::ThreadName;
    use std::cell::RefCell;

    #[test]
    fn fresh_reports_reach_the_sink_with_their_session_and_unread_count() {
        let key = SessionKey::Thread(ThreadName::parse("side").unwrap());
        let report = Report {
            worker: "w1".into(),
            status: "done".into(),
            at: String::new(),
            text: "finished".into(),
        };
        let delivered = RefCell::new(Vec::new());
        let sink = inbox_sink(key.clone(), |key, fresh, unread| {
            delivered.borrow_mut().push((key.clone(), fresh, unread));
        });
        sink(HostEvent::Repaint);
        sink(HostEvent::Fresh {
            fresh: vec![report.clone()],
            unread: 3,
        });
        assert_eq!(*delivered.borrow(), [(key, vec![report], 3)]);
    }
}
