use ralph_server::chat::serve_proto::{encode_frame, ServerMsg};
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};

pub(super) type Writer = Arc<Mutex<UnixStream>>;

pub(super) fn write_frame(writer: &Writer, id: u64, msg: &ServerMsg) -> std::io::Result<()> {
    writer
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .write_all(encode_frame(id, msg).as_bytes())
}

#[derive(Default)]
pub(super) struct Clients {
    attached: Mutex<Vec<(u64, Writer)>>,
    next: Mutex<u64>,
}

impl Clients {
    fn attached(&self) -> std::sync::MutexGuard<'_, Vec<(u64, Writer)>> {
        self.attached
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(super) fn attach(&self, writer: &Writer) -> u64 {
        let mut next = self
            .next
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *next += 1;
        self.attached().push((*next, Arc::clone(writer)));
        *next
    }

    pub(super) fn detach(&self, client: u64) {
        self.attached().retain(|(id, _)| *id != client);
    }

    pub(super) fn push(&self, msg: &ServerMsg) {
        let snapshot = self.attached().clone();
        for (client, writer) in snapshot {
            if write_frame(&writer, 0, msg).is_err() {
                self.detach(client);
            }
        }
    }
}
