use super::empty_submit;
use super::{ChatSection, EditorSetting, WORKER_QUIET_SECS};
use ralph_core::chat::{ResetLimits, RESET_TOKENS, RESET_TURNS};
use ralph_core::chat_status::EmptySubmit;

fn positive_setting(value: Option<&toml::Value>, key: &str, default: u64) -> (u64, Option<String>) {
    match value.map(toml::Value::as_integer) {
        None => (default, None),
        Some(Some(n)) if n > 0 => (n.unsigned_abs(), None),
        Some(_) => (
            default,
            Some(format!(
                "[chat] {key} must be a positive integer; using {default}"
            )),
        ),
    }
}

impl ChatSection {
    pub fn worker_quiet_setting(&self) -> (u64, Option<String>) {
        positive_setting(
            self.worker_quiet_secs.as_ref(),
            "worker_quiet_secs",
            WORKER_QUIET_SECS,
        )
    }

    pub fn reset_limits_setting(&self) -> (ResetLimits, Vec<String>) {
        let (turns, turns_warning) =
            positive_setting(self.reset_turns.as_ref(), "reset_turns", RESET_TURNS);
        let (tokens, tokens_warning) =
            positive_setting(self.reset_tokens.as_ref(), "reset_tokens", RESET_TOKENS);
        (
            ResetLimits { turns, tokens },
            turns_warning.into_iter().chain(tokens_warning).collect(),
        )
    }

    pub fn empty_submit_setting(&self) -> (EmptySubmit, Option<String>) {
        empty_submit::resolve_empty_submit(
            self.empty_submit.as_deref(),
            self.empty_prompt.as_deref(),
        )
    }

    pub fn editor_setting(&self) -> (Option<&[String]>, Option<String>) {
        match &self.editor {
            Some(EditorSetting::Argv(argv)) => (Some(argv), None),
            Some(EditorSetting::Invalid(_)) => (
                None,
                Some(
                    "[chat] editor must be a list of strings; using $VISUAL, $EDITOR or vi"
                        .to_string(),
                ),
            ),
            None => (None, None),
        }
    }
}
