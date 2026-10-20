use ralph_core::chat_status::EmptySubmit;

pub fn resolve_empty_submit(
    mode: Option<&str>,
    phrase: Option<&str>,
) -> (EmptySubmit, Option<String>) {
    let phrase = phrase.map(str::trim).filter(|phrase| !phrase.is_empty());
    let build: fn(String) -> EmptySubmit = match mode {
        None | Some("inbox") => return (EmptySubmit::Inbox, None),
        Some("inbox-first") => EmptySubmit::InboxFirst,
        Some("phrase") => EmptySubmit::Phrase,
        Some(other) => {
            return (
                EmptySubmit::Inbox,
                Some(format!(
                    "[chat] empty_submit = \"{other}\" is unknown; using inbox"
                )),
            )
        }
    };
    match phrase {
        Some(phrase) => (build(phrase.to_string()), None),
        None => (
            EmptySubmit::Inbox,
            Some(format!(
                "[chat] empty_submit = \"{}\" needs a non-empty empty_prompt; using inbox",
                mode.unwrap_or_default()
            )),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_inbox_mode_is_inbox_without_warning() {
        assert_eq!(resolve_empty_submit(None, None), (EmptySubmit::Inbox, None));
        assert_eq!(
            resolve_empty_submit(Some("inbox"), Some("go")),
            (EmptySubmit::Inbox, None)
        );
    }

    #[test]
    fn phrase_modes_use_the_trimmed_phrase() {
        assert_eq!(
            resolve_empty_submit(Some("inbox-first"), Some(" go ")),
            (EmptySubmit::InboxFirst("go".into()), None)
        );
        assert_eq!(
            resolve_empty_submit(Some("phrase"), Some("go")),
            (EmptySubmit::Phrase("go".into()), None)
        );
    }

    #[test]
    fn unknown_mode_warns_and_uses_inbox() {
        let (empty, warning) = resolve_empty_submit(Some("bogus"), Some("go"));
        assert_eq!(empty, EmptySubmit::Inbox);
        assert!(warning.unwrap().contains("\"bogus\" is unknown"));
    }

    #[test]
    fn phrase_modes_without_a_phrase_warn_and_use_inbox() {
        for mode in ["inbox-first", "phrase"] {
            for phrase in [None, Some(""), Some("  ")] {
                let (empty, warning) = resolve_empty_submit(Some(mode), phrase);
                assert_eq!(empty, EmptySubmit::Inbox);
                assert!(warning.unwrap().contains(mode));
            }
        }
    }
}
