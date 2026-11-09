use crate::cli::HookBridgeArgs;
use ralph_core::chat_socket::encode_line;
use ralph_core::worker::{
    hook_output, parse_hook_input, read_outcome, BridgeAsk, HookOutcome, BRIDGE_TIMEOUT,
};
use server::LINE_LIMIT_BYTES;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

pub(crate) mod server;

#[cfg(test)]
mod tests;

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
