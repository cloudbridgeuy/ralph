pub const SEPARATOR: &str = "──── write below ────";

pub fn editor_prefill(reply: &str, draft: &str) -> String {
    format!("{reply}\n{SEPARATOR}\n{draft}")
}

pub fn draft_below_separator(file: &str) -> String {
    let lines: Vec<&str> = file.lines().collect();
    let start = lines
        .iter()
        .rposition(|line| line.trim() == SEPARATOR)
        .map_or(0, |index| index + 1);
    lines[start..].join("\n").trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_text_below_separator() {
        let file = format!("reply\n{SEPARATOR}\nmy draft\n");
        assert_eq!(draft_below_separator(&file), "my draft");
    }

    #[test]
    fn last_separator_wins() {
        let file = format!("a\n{SEPARATOR}\nb\n{SEPARATOR}\nc\n");
        assert_eq!(draft_below_separator(&file), "c");
    }

    #[test]
    fn whole_file_without_separator() {
        assert_eq!(draft_below_separator("just text\n\n"), "just text");
    }

    #[test]
    fn edits_above_are_ignored() {
        let file = format!("edited reply\n{SEPARATOR}\nd");
        assert_eq!(draft_below_separator(&file), "d");
    }

    #[test]
    fn empty_draft_is_empty() {
        assert_eq!(draft_below_separator(&editor_prefill("reply", "")), "");
    }

    #[test]
    fn multiline_draft_roundtrips() {
        let prefill = editor_prefill("reply\nmore", "line1\nline2");
        assert_eq!(draft_below_separator(&prefill), "line1\nline2");
    }
}
