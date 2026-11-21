use super::clients::{write_frame, Writer};
use ralph_core::turn_event::{TurnEvent, TurnSink};
use ralph_server::chat::serve_proto::ServerMsg;
use std::sync::Mutex;

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
