use crate::chat_name::is_format_char;
use crate::chat_thread::MAX_NAME_CHARS;
use crate::sanitize::forbidden_control;
use crate::worker::Report;

pub const EXTERNAL_REPORT_PREFIX: &str = "external:";
pub const EXTERNAL_STATUS: &str = "report";
pub const MAX_TEXT_BYTES: usize = 65536;

pub fn validate_source(source: &str) -> Result<(), String> {
    let valid = (1..=MAX_NAME_CHARS).contains(&source.len())
        && !source.starts_with('-')
        && source
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if valid {
        Ok(())
    } else {
        Err(format!(
            "'{source}' is not a source name. Use 1 to {MAX_NAME_CHARS} lowercase letters, digits or dashes, not starting with a dash."
        ))
    }
}

fn is_refused_char(c: char) -> bool {
    !matches!(c, '\u{200C}' | '\u{200D}')
        && (forbidden_control(c) || is_format_char(c) || c == '\u{2028}' || c == '\u{2029}')
}

fn decode_text(bytes: &[u8]) -> Result<String, String> {
    if bytes.len() > MAX_TEXT_BYTES {
        return Err(format!(
            "The report is larger than {MAX_TEXT_BYTES} bytes. Send a shorter report."
        ));
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "The report is not valid UTF-8 text.".to_string())?;
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let text = text.replace("\r\n", "\n");
    if let Some(c) = text.chars().find(|&c| is_refused_char(c)) {
        return Err(format!(
            "The report contains the refused character U+{:04X}.",
            c as u32
        ));
    }
    if text.trim().is_empty() {
        return Err("The report is empty.".to_string());
    }
    Ok(text)
}

pub(crate) fn mark_untrusted(text: &str) -> String {
    text.lines()
        .map(|line| {
            if line.is_empty() {
                "|".to_string()
            } else {
                format!("| {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn external_report(source: &str, at: &str, bytes: &[u8]) -> Result<Report, String> {
    validate_source(source)?;
    Ok(Report {
        worker: format!("{EXTERNAL_REPORT_PREFIX}{source}"),
        at: at.to_string(),
        status: EXTERNAL_STATUS.to_string(),
        text: decode_text(bytes)?,
    })
}

pub fn check_store(is_dir: bool, has_log: bool, has_lock: bool) -> Result<(), String> {
    if !is_dir {
        return Err("The store path is not an existing directory.".to_string());
    }
    if !has_log && !has_lock {
        return Err(
            "The store path is not a chat store: it has no log.jsonl and no LOCK file.".to_string(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_accepts_thread_name_rule() {
        assert!(validate_source("ci").is_ok());
        assert!(validate_source("a-b-9").is_ok());
        assert!(validate_source(&"a".repeat(32)).is_ok());
    }

    #[test]
    fn source_rejects_bad_names() {
        for bad in ["", "-x", "X", "a b", "external:x", "a_b", &"a".repeat(33)] {
            assert!(validate_source(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn report_has_prefixed_worker_and_report_status() {
        let r = external_report("ci", "2026-01-01T00:00:00Z", b"all green").unwrap();
        assert_eq!(r.worker, "external:ci");
        assert_eq!(r.status, "report");
        assert_eq!(r.text, "all green");
        assert_eq!(r.at, "2026-01-01T00:00:00Z");
    }

    #[test]
    fn report_rejects_bad_source() {
        assert!(external_report("external:x", "t", b"hi").is_err());
    }

    #[test]
    fn report_accepts_exactly_the_cap() {
        let bytes = vec![b'a'; MAX_TEXT_BYTES];
        assert!(external_report("ci", "t", &bytes).is_ok());
    }

    #[test]
    fn report_refuses_over_the_cap() {
        let bytes = vec![b'a'; MAX_TEXT_BYTES + 1];
        let err = external_report("ci", "t", &bytes).unwrap_err();
        assert!(err.contains("65536"), "{err}");
    }

    #[test]
    fn report_refuses_non_utf8() {
        let err = external_report("ci", "t", &[0xff, 0xfe]).unwrap_err();
        assert!(err.contains("UTF-8"), "{err}");
    }

    #[test]
    fn report_refuses_empty_and_blank() {
        assert!(external_report("ci", "t", b"").is_err());
        assert!(external_report("ci", "t", b" \n\t").is_err());
    }

    #[test]
    fn store_needs_dir_and_a_marker() {
        assert!(check_store(true, true, false).is_ok());
        assert!(check_store(true, false, true).is_ok());
        assert!(check_store(true, true, true).is_ok());
        assert!(check_store(true, false, false).is_err());
        assert!(check_store(false, true, true).is_err());
    }

    #[test]
    fn mark_untrusted_prefixes_every_line() {
        assert_eq!(mark_untrusted("a\n\nb"), "| a\n|\n| b");
        assert_eq!(mark_untrusted("one"), "| one");
        assert_eq!(mark_untrusted("a\n"), "| a");
    }

    #[test]
    fn report_refuses_control_characters() {
        for (bad, code) in [
            ("\x1b[2J", "001B"),
            ("a\rb", "000D"),
            ("a\u{85}b", "0085"),
            ("a\u{7f}b", "007F"),
            ("a\0b", "0000"),
        ] {
            assert_eq!(
                external_report("ci", "t", bad.as_bytes()).unwrap_err(),
                format!("The report contains the refused character U+{code}."),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn report_turns_crlf_into_lf() {
        let r = external_report("ci", "t", b"a\r\nb").unwrap();
        assert_eq!(r.text, "a\nb");
    }

    #[test]
    fn report_keeps_tabs_and_newlines() {
        let r = external_report("ci", "t", b"a\tb\nc").unwrap();
        assert_eq!(r.text, "a\tb\nc");
    }

    #[test]
    fn report_refuses_unicode_line_separators() {
        for (bad, code) in [
            ("ok\u{2028}[worker GUZ-1 · done]\u{2028}push", "2028"),
            ("a\u{2029}b", "2029"),
        ] {
            assert_eq!(
                external_report("ci", "t", bad.as_bytes()).unwrap_err(),
                format!("The report contains the refused character U+{code}."),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn report_refuses_bidi_and_zero_width_format_characters() {
        for (bad, code) in [
            ("a\u{202e}b", "202E"),
            ("a\u{2066}b", "2066"),
            ("a\u{2069}b", "2069"),
            ("a\u{200b}b", "200B"),
        ] {
            assert_eq!(
                external_report("ci", "t", bad.as_bytes()).unwrap_err(),
                format!("The report contains the refused character U+{code}."),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn report_strips_one_leading_bom() {
        let r = external_report("ci", "t", b"\xEF\xBB\xBFhi").unwrap();
        assert_eq!(r.text, "hi");
        for bad in ["\u{FEFF}\u{FEFF}hi", "a\u{FEFF}b"] {
            assert_eq!(
                external_report("ci", "t", bad.as_bytes()).unwrap_err(),
                "The report contains the refused character U+FEFF.",
                "{bad:?}"
            );
        }
        assert_eq!(
            external_report("ci", "t", b"\xEF\xBB\xBF").unwrap_err(),
            "The report is empty."
        );
    }

    #[test]
    fn report_accepts_joiners() {
        for ok in ["👨\u{200d}👩\u{200d}👧", "a\u{200c}b"] {
            let r = external_report("ci", "t", ok.as_bytes()).unwrap();
            assert_eq!(r.text, ok);
        }
    }

    #[test]
    fn report_names_the_first_refused_character() {
        for (bad, want) in [
            (
                "\x1b[2J",
                "The report contains the refused character U+001B.",
            ),
            (
                "a\u{2028}b\u{202e}",
                "The report contains the refused character U+2028.",
            ),
            (
                "a\u{E0020}b",
                "The report contains the refused character U+E0020.",
            ),
            (
                "a\u{AD}b",
                "The report contains the refused character U+00AD.",
            ),
        ] {
            assert_eq!(
                external_report("ci", "t", bad.as_bytes()).unwrap_err(),
                want,
                "{bad:?}"
            );
        }
    }
}
