use ralph_core::turn_event::{TurnEvent, TurnSink};
use ralph_server::chat::serve_proto::{encode_frame, ServerMsg};
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::Mutex;

#[derive(Default)]
pub(super) struct SocketSink {
    bound: Mutex<Option<(u64, UnixStream)>>,
}

impl SocketSink {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<(u64, UnixStream)>> {
        self.bound
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(super) fn bind(&self, id: u64, stream: UnixStream) {
        *self.lock() = Some((id, stream));
    }

    pub(super) fn unbind(&self) {
        self.lock().take();
    }
}

impl TurnSink for SocketSink {
    fn emit(&self, event: TurnEvent) {
        if let Some((id, mut stream)) = self.lock().as_ref().map(|(id, stream)| (*id, stream)) {
            let frame = encode_frame(id, &ServerMsg::Turn { event });
            let _ = stream.write_all(frame.as_bytes());
        }
    }
}
