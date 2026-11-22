use ralph_server::chat::serve_proto::{encode_frame, ServerMsg};
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub(super) type Writer = Arc<Mutex<dyn Write + Send>>;

pub(super) fn write_frame(writer: &Writer, id: u64, msg: &ServerMsg) -> std::io::Result<()> {
    writer
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .write_all(encode_frame(id, msg).as_bytes())
}

#[derive(Clone)]
struct Client {
    id: u64,
    writer: Writer,
    inbox_push: bool,
}

#[derive(Default)]
pub(super) struct Clients {
    attached: Mutex<Vec<Client>>,
    next: AtomicU64,
}

impl Clients {
    fn attached(&self) -> std::sync::MutexGuard<'_, Vec<Client>> {
        self.attached
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(super) fn greet(
        &self,
        writer: &Writer,
        id: u64,
        hello: &ServerMsg,
        inbox_push: bool,
    ) -> std::io::Result<u64> {
        let mut stream = writer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let client = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        self.attached().push(Client {
            id: client,
            writer: Arc::clone(writer),
            inbox_push,
        });
        if let Err(error) = stream.write_all(encode_frame(id, hello).as_bytes()) {
            self.detach(client);
            return Err(error);
        }
        Ok(client)
    }

    pub(super) fn detach(&self, client: u64) {
        self.attached().retain(|peer| peer.id != client);
    }

    pub(super) fn push(&self, msg: &ServerMsg) {
        let inbox = matches!(msg, ServerMsg::Inbox { .. });
        let recipients: Vec<_> = self
            .attached()
            .iter()
            .filter(|peer| !inbox || peer.inbox_push)
            .cloned()
            .collect();
        let frame = encode_frame(0, msg);
        for peer in recipients {
            let mut stream = peer
                .writer
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if stream.write_all(frame.as_bytes()).is_err() {
                self.detach(peer.id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::chat_process::SessionKey;
    use ralph_core::worker::Report;
    use ralph_server::chat::serve_proto::{decode_frame, hello_reply, inbox_frame, VERSION};

    #[test]
    fn inbox_push_goes_to_every_opted_in_client_after_hello() {
        let clients = Clients::default();
        let buffers = [false, true, true].map(|opted_in| {
            let buffer = Arc::new(Mutex::new(Vec::<u8>::new()));
            let writer: Writer = buffer.clone();
            clients
                .greet(&writer, 1, &hello_reply(VERSION), opted_in)
                .unwrap();
            buffer
        });
        let report = Report {
            worker: "w1".into(),
            status: "done".into(),
            text: "finished".into(),
            at: "now".into(),
        };
        clients.push(&inbox_frame(&SessionKey::Main, report, 1));
        for (buffer, expected) in buffers.iter().zip([1, 2, 2]) {
            let bytes = buffer.lock().unwrap();
            let frames: Vec<_> = std::str::from_utf8(&bytes)
                .unwrap()
                .lines()
                .map(|line| decode_frame::<ServerMsg>(line).unwrap())
                .collect();
            assert_eq!(frames.len(), expected);
            assert_eq!(frames[0].id, 1);
            assert!(matches!(frames[0].body, ServerMsg::Hello { .. }));
            if expected == 2 {
                assert_eq!(frames[1].id, 0);
                assert!(matches!(
                    frames[1].body,
                    ServerMsg::Inbox {
                        thread: None,
                        unread: 1,
                        ..
                    }
                ));
            }
        }
    }
}
