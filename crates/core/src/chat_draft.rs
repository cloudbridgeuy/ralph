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

pub fn editor_argv(
    config: Option<&[String]>,
    visual: Option<&str>,
    editor: Option<&str>,
) -> (Vec<String>, Option<String>) {
    match config {
        Some(argv) if !argv.is_empty() => (argv.to_vec(), None),
        _ => {
            let warning =
                config.map(|_| "[chat] editor is empty; using $VISUAL, $EDITOR or vi".to_string());
            let program = [visual, editor]
                .into_iter()
                .flatten()
                .find(|value| !value.is_empty())
                .unwrap_or("vi");
            (vec![program.to_string()], warning)
        }
    }
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

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| part.to_string()).collect()
    }

    #[test]
    fn configured_editor_wins_without_warning() {
        let config = argv(&["nvim", "-c", "startinsert"]);
        assert_eq!(
            editor_argv(Some(&config), Some("code"), Some("nano")),
            (config, None)
        );
    }

    #[test]
    fn empty_config_warns_and_falls_back() {
        let (argv, warning) = editor_argv(Some(&[]), Some("code"), None);
        assert_eq!(argv, vec!["code".to_string()]);
        assert!(warning.is_some());
    }

    #[test]
    fn visual_beats_editor() {
        assert_eq!(
            editor_argv(None, Some("code"), Some("nano")),
            (argv(&["code"]), None)
        );
    }

    #[test]
    fn editor_used_when_visual_unset_or_empty() {
        let expected = (argv(&["nano"]), None);
        assert_eq!(editor_argv(None, None, Some("nano")), expected);
        assert_eq!(editor_argv(None, Some(""), Some("nano")), expected);
    }

    #[test]
    fn defaults_to_vi() {
        assert_eq!(editor_argv(None, None, None), (argv(&["vi"]), None));
    }
}
