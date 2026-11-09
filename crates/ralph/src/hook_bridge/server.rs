use crate::permission_queue::PermissionQueue;
use ralph_core::chat_socket::{decode_line, encode_line};
use ralph_core::worker::{
    bridge_wait, hook_content, hook_request_id, BridgeAnswer, BridgeAsk, BridgeWait,
};
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

pub(super) const LINE_LIMIT_BYTES: u64 = 1 << 20;
const ASK_READ_TIMEOUT: Duration = Duration::from_secs(30);
const HOOK_POLL: Duration = Duration::from_millis(250);

pub(crate) struct HookReply {
    pub(super) id: String,
    pub(super) answer: mpsc::Sender<String>,
}

pub(crate) type HookQueue = PermissionQueue<HookReply>;

pub(crate) fn deliver(reply: HookReply, option_id: String, request_id: &str) -> Result<(), String> {
    reply
        .answer
        .send(option_id)
        .map_err(|_| format!("stale request {request_id}: the hook already denied the command"))
}

pub(crate) struct HookSocket {
    listener: UnixListener,
    path: PathBuf,
    closed: Arc<AtomicBool>,
}

impl HookSocket {
    pub(crate) fn bind(path: &Path) -> io::Result<Self> {
        fs::remove_file(path).ok();
        let listener = UnixListener::bind(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        Ok(Self {
            listener,
            path: path.to_path_buf(),
            closed: Arc::new(AtomicBool::new(false)),
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn serve(&self, queue: Arc<HookQueue>) -> io::Result<()> {
        let listener = self.listener.try_clone()?;
        let closed = Arc::clone(&self.closed);
        let counter = Arc::new(AtomicU64::new(0));
        thread::spawn(move || {
            for stream in listener.incoming() {
                if closed.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let (queue, counter) = (Arc::clone(&queue), Arc::clone(&counter));
                thread::spawn(move || {
                    if let Err(e) = answer_connection(&stream, &queue, &counter) {
                        crate::warn::warn(format!("cursor hook bridge: {e}"));
                    }
                });
            }
        });
        Ok(())
    }
}

fn answer_connection(
    stream: &UnixStream,
    queue: &HookQueue,
    counter: &AtomicU64,
) -> io::Result<()> {
    stream.set_read_timeout(Some(ASK_READ_TIMEOUT))?;
    let mut line = String::new();
    BufReader::new(stream.take(LINE_LIMIT_BYTES)).read_line(&mut line)?;
    let request: BridgeAsk = decode_line(&line).map_err(io::Error::other)?;
    let id = hook_request_id(&request.tool_use_id, counter.fetch_add(1, Ordering::SeqCst));
    let (answer, chosen) = mpsc::channel();
    let content = hook_content(&id, &request);
    queue.enqueue(
        HookReply {
            id: id.clone(),
            answer,
        },
        content,
    );
    let option = wait_for_answer(stream, &chosen);
    drop(chosen);
    let Some(option_id) = option else {
        queue.settle(|reply| reply.id == id);
        return Ok(());
    };
    let mut writer = stream;
    writer.write_all(encode_line(&BridgeAnswer { option_id }).as_bytes())?;
    writer.flush()
}

fn wait_for_answer(stream: &UnixStream, chosen: &mpsc::Receiver<String>) -> Option<String> {
    let started = Instant::now();
    loop {
        match chosen.recv_timeout(HOOK_POLL) {
            Ok(option_id) => return Some(option_id),
            Err(mpsc::RecvTimeoutError::Disconnected) => return None,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if bridge_wait(started.elapsed(), hook_connected(stream)) == BridgeWait::GiveUp {
                    return None;
                }
            }
        }
    }
}

fn hook_connected(stream: &UnixStream) -> bool {
    if stream.set_nonblocking(true).is_err() {
        return false;
    }
    let read = (&*stream).read(&mut [0u8; 1]);
    let connected = matches!(&read, Ok(n) if *n > 0)
        || matches!(&read, Err(e) if e.kind() == io::ErrorKind::WouldBlock);
    stream.set_nonblocking(false).is_ok() && connected
}

impl Drop for HookSocket {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
        UnixStream::connect(&self.path).ok();
        fs::remove_file(&self.path).ok();
    }
}
