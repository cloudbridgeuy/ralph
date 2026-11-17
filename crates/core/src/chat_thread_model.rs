use crate::chat_thread::ThreadMeta;

pub fn with_model(meta: &ThreadMeta, model: Option<&str>) -> Option<ThreadMeta> {
    model
        .filter(|model| meta.model.as_deref() != Some(*model))
        .map(|model| ThreadMeta {
            model: Some(model.to_string()),
            ..meta.clone()
        })
}

pub fn build_model_args(model: Option<&str>) -> Vec<String> {
    model.map_or_else(Vec::new, |model| {
        vec!["--model".to_string(), model.to_string()]
    })
}

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

    #[test]
    fn thread_meta_round_trips_model_and_reads_files_without_one() {
        let text = r#"{"base":1,"created":"t","model":"haiku"}"#;
        let meta: ThreadMeta = serde_json::from_str(text).unwrap();
        assert_eq!(meta.model.as_deref(), Some("haiku"));
        assert_eq!(serde_json::to_string(&meta).unwrap(), text);
        let old: ThreadMeta = serde_json::from_str(r#"{"base":1,"created":"t"}"#).unwrap();
        assert_eq!(old.model, None);
    }

    fn meta(model: Option<&str>) -> ThreadMeta {
        ThreadMeta {
            base: 1,
            created: "t".to_string(),
            merged: None,
            parked: None,
            auto: None,
            model: model.map(str::to_string),
            closed: false,
        }
    }

    #[test]
    fn with_model_changes_only_when_a_different_model_is_given() {
        assert_eq!(with_model(&meta(None), None), None);
        assert_eq!(with_model(&meta(Some("opus")), None), None);
        assert_eq!(with_model(&meta(Some("opus")), Some("opus")), None);
        assert_eq!(
            with_model(&meta(Some("opus")), Some("haiku")),
            Some(meta(Some("haiku")))
        );
        assert_eq!(
            with_model(&meta(None), Some("haiku")),
            Some(meta(Some("haiku")))
        );
    }

    #[test]
    fn model_args_are_a_flag_pair_only_when_a_model_is_set() {
        assert_eq!(build_model_args(None), Vec::<String>::new());
        assert_eq!(build_model_args(Some("haiku")), ["--model", "haiku"]);
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
