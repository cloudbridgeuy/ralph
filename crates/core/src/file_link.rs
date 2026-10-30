//! OSC 8 `file://` hyperlinks for local paths shown in the terminal.

use std::path::Path;

use crate::schedule::escape_controls;

const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

pub fn file_url(path: &Path) -> String {
    let mut url = String::from("file://");
    for &byte in path.as_os_str().as_encoded_bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
            url.push(char::from(byte));
        } else {
            url.push_str(&format!("%{byte:02X}"));
        }
    }
    url
}

pub fn path_line(path: &Path) -> String {
    let shown = escape_controls(&path.display().to_string());
    if path.is_absolute() {
        format!(
            "{DIM}\x1b]8;;{}\x1b\\{shown}\x1b]8;;\x1b\\{RESET}\n",
            file_url(path)
        )
    } else {
        format!("{DIM}{shown}{RESET}\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_url_encodes_space_hash_question_percent_and_non_ascii() {
        assert_eq!(
            file_url(Path::new("/tmp/a b#c?d%é.png")),
            "file:///tmp/a%20b%23c%3Fd%25%C3%A9.png"
        );
    }

    #[test]
    fn file_url_keeps_slashes_and_unreserved_chars() {
        assert_eq!(
            file_url(Path::new("/Users/me/A-z_0.9~/x.png")),
            "file:///Users/me/A-z_0.9~/x.png"
        );
    }

    const HOSTILE: &str = "/tmp/a\x1b]8;;evil\x1b\\b\x07\x7f\u{9b}.png";

    #[test]
    fn file_url_percent_encodes_control_bytes() {
        assert_eq!(
            file_url(Path::new(HOSTILE)),
            "file:///tmp/a%1B%5D8%3B%3Bevil%1B%5Cb%07%7F%C2%9B.png"
        );
    }

    #[test]
    fn path_line_shows_control_chars_escaped() {
        let line = path_line(Path::new(HOSTILE));
        let url = file_url(Path::new(HOSTILE));
        assert_eq!(
            line,
            format!(
                "\x1b[2m\x1b]8;;{url}\x1b\\/tmp/a\\x1b]8;;evil\\x1b\\b\\x07\\x7f\\x9b.png\x1b]8;;\x1b\\\x1b[0m\n"
            )
        );
        assert_eq!(line.matches('\x1b').count(), 6, "{line:?}");
        assert!(!line.contains('\x07') && !line.contains('\x7f') && !line.contains('\u{9b}'));
    }

    #[test]
    fn path_line_wraps_plain_path_in_dim_osc8_link() {
        assert_eq!(
            path_line(Path::new("/tmp/a b.png")),
            "\x1b[2m\x1b]8;;file:///tmp/a%20b.png\x1b\\/tmp/a b.png\x1b]8;;\x1b\\\x1b[0m\n"
        );
    }

    #[test]
    fn path_line_escapes_bidi_and_format_chars() {
        let path = Path::new("/tmp/a\u{202e}gnp\u{200b}\u{2066}\u{feff}.exe");
        let line = path_line(path);
        assert!(
            line.contains("/tmp/a\\u{202e}gnp\\u{200b}\\u{2066}\\u{feff}.exe\x1b]8;;"),
            "{line:?}"
        );
        assert!(line.contains("file:///tmp/a%E2%80%AEgnp%E2%80%8B%E2%81%A6%EF%BB%BF.exe"));
    }

    #[test]
    fn relative_path_line_is_dim_escaped_text_without_link() {
        assert_eq!(
            path_line(Path::new("img/a\u{202e}.png")),
            "\x1b[2mimg/a\\u{202e}.png\x1b[0m\n"
        );
    }
}
