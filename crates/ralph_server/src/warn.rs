use std::collections::BTreeSet;
use std::fmt::Display;
use std::io::{self, Write};
use std::sync::{Mutex, OnceLock};

pub const WARNING_PREFIX: &str = "Warning:";

pub type WarnSink = fn(&str);

static SINK: OnceLock<WarnSink> = OnceLock::new();

pub fn register_sink(sink: WarnSink) {
    let _ = SINK.set(sink);
}

fn emit(message: &str, sink: Option<&dyn Fn(&str)>, out: &mut impl Write) -> io::Result<()> {
    match sink {
        Some(sink) => {
            sink(message);
            Ok(())
        }
        None => out.write_all(format!("{WARNING_PREFIX} {message}\n").as_bytes()),
    }
}

pub fn warn(message: impl Display) {
    let sink = SINK.get().map(|sink| sink as &dyn Fn(&str));
    emit(&message.to_string(), sink, &mut io::stderr()).ok();
}

static NOTICE_SINK: OnceLock<WarnSink> = OnceLock::new();

pub fn register_notice_sink(sink: WarnSink) {
    let _ = NOTICE_SINK.set(sink);
}

pub fn notice(message: impl Display) {
    match NOTICE_SINK.get() {
        Some(sink) => sink(&message.to_string()),
        None => writeln!(io::stderr(), "{message}").unwrap_or(()),
    }
}

static SEEN: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

fn first_sighting(seen: &mut BTreeSet<String>, key: &str) -> bool {
    seen.insert(key.to_string())
}

pub fn warn_once(key: &str, message: impl Display) {
    let fresh = match SEEN.lock() {
        Ok(mut seen) => first_sighting(&mut seen, key),
        Err(_) => true,
    };
    if fresh {
        warn(message);
    }
}

pub fn warn_if_err<T, E: Display>(result: Result<T, E>, context: impl Display) {
    if let Err(e) = result {
        warn(format!("{context}: {e}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn emit_without_a_sink_still_prints() {
        let mut out = Vec::new();
        emit("x", None, &mut out).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "Warning: x\n");
    }

    #[test]
    fn emit_with_a_sink_writes_nothing_itself() {
        let seen = RefCell::new(Vec::new());
        let sink = |message: &str| seen.borrow_mut().push(message.to_string());
        let mut out = Vec::new();
        emit("x", Some(&sink), &mut out).unwrap();
        assert_eq!(seen.into_inner(), ["x"]);
        assert!(out.is_empty());
    }

    #[test]
    fn first_sighting_is_true_once_per_key() {
        let mut seen = BTreeSet::new();
        assert!(first_sighting(&mut seen, "a"));
        assert!(!first_sighting(&mut seen, "a"));
        assert!(first_sighting(&mut seen, "b"));
    }

    #[test]
    fn test_warn_if_err_with_ok_does_not_panic() {
        let result: Result<i32, io::Error> = Ok(42);
        warn_if_err(result, "This should not print");
    }

    #[test]
    fn test_warn_if_err_with_err_does_not_panic() {
        let result: Result<i32, io::Error> =
            Err(io::Error::new(io::ErrorKind::NotFound, "test error"));
        warn_if_err(result, "Test context");
    }
}
