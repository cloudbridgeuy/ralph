pub fn validate_thread_model(model: &str) -> Result<(), String> {
    let known_id = model
        .strip_prefix("claude-")
        .is_some_and(|rest| !rest.is_empty());
    if matches!(model, "sonnet" | "opus" | "haiku")
        || (known_id && !model.chars().any(char::is_whitespace))
    {
        Ok(())
    } else {
        Err(format!(
            "'{model}' is not a thread model. Use sonnet, opus, haiku or a claude-* model id."
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_thread::ThreadMeta;

    #[test]
    fn thread_meta_round_trips_model_and_reads_files_without_one() {
        let text = r#"{"base":1,"created":"t","model":"haiku"}"#;
        let meta: ThreadMeta = serde_json::from_str(text).unwrap();
        assert_eq!(meta.model.as_deref(), Some("haiku"));
        assert_eq!(serde_json::to_string(&meta).unwrap(), text);
        let old: ThreadMeta = serde_json::from_str(r#"{"base":1,"created":"t"}"#).unwrap();
        assert_eq!(old.model, None);
    }

    #[test]
    fn thread_models_are_aliases_or_claude_ids() {
        for ok in ["sonnet", "opus", "haiku", "claude-haiku-4-5", "claude-x"] {
            assert_eq!(validate_thread_model(ok), Ok(()), "{ok}");
        }
        for bad in [
            "",
            "gpt-5",
            "Opus",
            "claude-",
            "claude",
            "claude-a b",
            "claude-a\tb",
            " haiku",
        ] {
            let err = validate_thread_model(bad).unwrap_err();
            assert!(err.contains("is not a thread model"), "{bad}: {err}");
        }
    }
}
