use ralph_core::meeting::{
    finish_relay, handle_request, MeetingContext, MeetingEvent, MeetingReply, Outcome,
};
use serde_json::Value;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

pub fn relay(socket: &Path, event: &MeetingEvent) -> io::Result<MeetingReply> {
    let mut stream = UnixStream::connect(socket)?;
    let mut line = serde_json::to_string(event).map_err(io::Error::other)?;
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply)?;
    if reply.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "moderator closed the connection without a reply",
        ));
    }
    serde_json::from_str(&reply).map_err(io::Error::other)
}

pub fn run_stdio_server(
    ctx: &MeetingContext,
    socket: &Path,
    input: impl BufRead,
    mut output: impl Write,
) -> io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                crate::warn::warn(format!("ignoring malformed JSON-RPC line: {e}"));
                continue;
            }
        };
        let response = match handle_request(ctx, &request) {
            Outcome::Silent => continue,
            Outcome::Respond(v) => v,
            Outcome::Relay { id, event } => {
                let reply = relay(socket, &event).map_err(|e| e.to_string());
                finish_relay(id, &event, reply)
            }
        };
        writeln!(output, "{response}")?;
        output.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::thread;

    #[test]
    fn relay_round_trips_through_listener() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("m.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            let event: MeetingEvent = serde_json::from_str(&line).unwrap();
            let MeetingEvent::AskHuman { question, .. } = event else {
                panic!("unexpected event");
            };
            let reply = MeetingReply {
                text: format!("answer to {question}"),
            };
            writeln!(stream, "{}", serde_json::to_string(&reply).unwrap()).unwrap();
        });

        let reply = relay(
            &socket,
            &MeetingEvent::AskHuman {
                from: "alice".into(),
                question: "why".into(),
            },
        )
        .unwrap();
        server.join().unwrap();
        assert_eq!(reply.text, "answer to why");
    }

    #[test]
    fn stdio_server_answers_requests_and_skips_notifications() {
        let ctx = MeetingContext {
            caller: "alice".into(),
            roster: vec!["alice".into(), "bob".into()],
        };
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"send_message","arguments":{"to":"zed","text":"x"}}}"#,
            "\n",
        );
        let mut out = Vec::new();
        run_stdio_server(&ctx, Path::new("/nonexistent"), input.as_bytes(), &mut out).unwrap();
        let lines: Vec<Value> = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1]["result"]["isError"], true);
    }
}
