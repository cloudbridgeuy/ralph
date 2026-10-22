use super::is_cursor_model;

pub fn resolve_cursor_model(
    spawn: Option<&str>,
    config: Result<Option<&str>, &str>,
) -> (Option<String>, Option<String>) {
    if let Some(model) = spawn {
        return (Some(model.to_string()), None);
    }
    match config {
        Err(error) => (
            None,
            Some(format!(
                "Warning: [cursor] model was skipped because the ralph config did not load ({error}). Used cursor's default model instead."
            )),
        ),
        Ok(None) => (None, None),
        Ok(Some(model)) if is_cursor_model(model) => (Some(model.to_string()), None),
        Ok(Some(model)) => (
            None,
            Some(format!(
                "[cursor] model '{model}' must be non-empty with no whitespace; using cursor's default model."
            )),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_model_resolution_prefers_spawn_then_config_then_default() {
        assert_eq!(
            resolve_cursor_model(Some("gpt-5"), Ok(Some("sonnet-4"))),
            (Some("gpt-5".to_string()), None)
        );
        assert_eq!(
            resolve_cursor_model(None, Ok(Some("sonnet-4"))),
            (Some("sonnet-4".to_string()), None)
        );
        assert_eq!(resolve_cursor_model(None, Ok(None)), (None, None));
        assert_eq!(
            resolve_cursor_model(
                None,
                Ok(Some("claude-opus-4-8[context=1m,effort=high,fast=false]"))
            ),
            (
                Some("claude-opus-4-8[context=1m,effort=high,fast=false]".to_string()),
                None
            )
        );
    }

    #[test]
    fn cursor_model_resolution_warns_on_invalid_config() {
        for bad in ["", "two words", " gpt-5"] {
            let (model, warning) = resolve_cursor_model(None, Ok(Some(bad)));
            assert_eq!(model, None);
            assert!(warning.unwrap().contains("[cursor] model"), "{bad}");
        }
        assert_eq!(
            resolve_cursor_model(Some("gpt-5"), Ok(Some(""))),
            (Some("gpt-5".to_string()), None)
        );
    }

    #[test]
    fn cursor_model_resolution_warns_when_config_fails_to_load() {
        let (model, warning) = resolve_cursor_model(None, Err("bad toml at line 3"));
        assert_eq!(model, None);
        let warning = warning.unwrap();
        assert!(warning.contains("bad toml at line 3"));
        assert!(warning.contains("default model"));
    }

    #[test]
    fn cursor_model_resolution_ignores_config_error_when_spawn_model_given() {
        assert_eq!(
            resolve_cursor_model(Some("gpt-5"), Err("boom")),
            (Some("gpt-5".to_string()), None)
        );
    }
}
