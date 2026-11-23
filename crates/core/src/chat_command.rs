use crate::chat_direct::{parse_direct, DirectLine};
use std::ops::Range;

pub fn leading_command(text: &str) -> Option<Range<usize>> {
    let trimmed = text.trim_start();
    let token = trimmed.split_whitespace().next()?;
    if !is_slash_command(token) && !matches!(parse_direct(token), Some(DirectLine::Message(_))) {
        return None;
    }
    let start = text.len() - trimmed.len();
    Some(start..start + token.len())
}

fn is_slash_command(token: &str) -> bool {
    let Some(name) = token.strip_prefix('/') else {
        return false;
    };
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|byte| byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::leading_command;

    #[test]
    fn leading_command_accepts_names_and_preserves_byte_offsets() {
        for (text, expected) in [
            ("/hold GUZ-1457", 0..5),
            ("/diff", 0..5),
            ("/merge", 0..6),
            (" \t/Custom_12-name args", 2..17),
            ("\n\u{2003}/x\targ", 4..6),
            ("/a\n/merge", 0..2),
        ] {
            assert_eq!(leading_command(text), Some(expected), "{text:?}");
        }
    }

    #[test]
    fn leading_command_accepts_worker_addresses_and_preserves_byte_offsets() {
        for (text, expected) in [
            ("@GUZ-1449 run tests", 0..9),
            ("@w1", 0..3),
            (" \t@Custom_12.name:1 args", 2..19),
            ("\n\u{2003}@x\targ", 4..6),
            ("@a\n@w2", 0..2),
        ] {
            assert_eq!(leading_command(text), Some(expected), "{text:?}");
        }
    }

    #[test]
    fn leading_command_rejects_paths_and_non_command_tokens() {
        for text in [
            "",
            " ",
            "/tmp/x",
            "//x",
            "/",
            "/ args",
            "text /hold",
            "/1name",
            "/_name",
            "/-name",
            "/é",
            "/hold!",
            "/hold.args",
            "/holdé",
            " /tmp/x args",
            "@",
            "@ args",
            "@@GUZ-1 hi",
            "@@@x",
            "hello @GUZ-1",
            "me@example.com",
            "email me@x.io now",
        ] {
            assert_eq!(leading_command(text), None, "{text:?}");
        }
    }
}
