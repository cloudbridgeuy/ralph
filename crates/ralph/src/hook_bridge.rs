use crate::cli::HookBridgeArgs;
use ralph_core::chat_socket::{decode_line, encode_line};
use ralph_core::worker::{
    hook_output, hook_request_id, parse_hook_input, read_outcome, BridgeAnswer, BridgeAsk,
    HookOutcome, BRIDGE_TIMEOUT,
};
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[cfg(test)]
mod tests;

const LINE_LIMIT_BYTES: u64 = 1 << 20;
const ASK_READ_TIMEOUT: Duration = Duration::from_secs(30);

pub fn execute_hook_bridge(args: HookBridgeArgs) -> Result<(), Box<dyn std::error::Error>> {
    let mut stdin = String::new();
    let outcome = match io::stdin()
        .take(LINE_LIMIT_BYTES)
        .read_to_string(&mut stdin)
    {
        Ok(_) => match parse_hook_input(&stdin) {
            Ok(input) => ask_bridge(
                &args.socket,
                &BridgeAsk {
                    tool_use_id: input.tool_use_id,
                    command: input.command,
                },
            ),
            Err(e) => HookOutcome::BadInput(e),
        },
        Err(e) => HookOutcome::BadInput(e.to_string()),
    };
    println!("{}", hook_output(&outcome));
    Ok(())
}

fn ask_bridge(socket: &Path, ask: &BridgeAsk) -> HookOutcome {
    read_outcome(send_ask(socket, ask))
}

fn send_ask(socket: &Path, ask: &BridgeAsk) -> io::Result<String> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(BRIDGE_TIMEOUT))?;
    stream.write_all(encode_line(ask).as_bytes())?;
    stream.flush()?;
    let mut line = String::new();
    BufReader::new(stream.take(LINE_LIMIT_BYTES)).read_line(&mut line)?;
    Ok(line)
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

    pub(crate) fn serve(
        &self,
        ask: impl Fn(String, BridgeAsk) -> Option<String> + Send + Sync + 'static,
    ) -> io::Result<()> {
        let listener = self.listener.try_clone()?;
        let closed = Arc::clone(&self.closed);
        let ask = Arc::new(ask);
        let counter = Arc::new(AtomicU64::new(0));
        thread::spawn(move || {
            for stream in listener.incoming() {
                if closed.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let (ask, counter) = (Arc::clone(&ask), Arc::clone(&counter));
                thread::spawn(move || {
                    if let Err(e) = answer_connection(&stream, ask.as_ref(), &counter) {
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
    ask: &(impl Fn(String, BridgeAsk) -> Option<String> + ?Sized),
    counter: &AtomicU64,
) -> io::Result<()> {
    stream.set_read_timeout(Some(ASK_READ_TIMEOUT))?;
    let mut line = String::new();
    BufReader::new(stream.take(LINE_LIMIT_BYTES)).read_line(&mut line)?;
    let request: BridgeAsk = decode_line(&line).map_err(io::Error::other)?;
    let id = hook_request_id(&request.tool_use_id, counter.fetch_add(1, Ordering::SeqCst));
    let Some(option_id) = ask(id, request) else {
        return Ok(());
    };
    let mut writer = stream;
    writer.write_all(encode_line(&BridgeAnswer { option_id }).as_bytes())?;
    writer.flush()
}

impl Drop for HookSocket {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
        UnixStream::connect(&self.path).ok();
        fs::remove_file(&self.path).ok();
    }
}
