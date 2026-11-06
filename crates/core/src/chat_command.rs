use std::ops::Range;

pub fn leading_command(text: &str) -> Option<Range<usize>> {
    let trimmed = text.trim_start();
    let token = trimmed.split_whitespace().next()?;
    let name = token.strip_prefix('/')?;
    let mut bytes = name.bytes();
    if !bytes.next()?.is_ascii_alphabetic()
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return None;
    }
    let start = text.len() - trimmed.len();
    Some(start..start + token.len())
}

#[cfg(test)]
mod tests {
    use super::leading_command;

    #[test]
    fn leading_command_accepts_names_and_preserves_byte_offsets() {
        for (text, expected) in [
            ("/hold GUZ-1457", 0..5),
            ("/fork", 0..5),
            ("/merge", 0..6),
            (" \t/Custom_12-name args", 2..17),
            ("\n\u{2003}/x\targ", 4..6),
            ("/a\n/merge", 0..2),
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
            "/fork.args",
            "/holdé",
            " /tmp/x args",
        ] {
            assert_eq!(leading_command(text), None, "{text:?}");
        }
    }
}
