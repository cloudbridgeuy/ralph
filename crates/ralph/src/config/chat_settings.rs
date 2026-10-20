use super::empty_submit;
use super::{ChatSection, EditorSetting, WORKER_QUIET_SECS};
use ralph_core::chat_status::EmptySubmit;

impl ChatSection {
    pub fn worker_quiet_setting(&self) -> (u64, Option<String>) {
        match self.worker_quiet_secs.as_ref().map(toml::Value::as_integer) {
            None => (WORKER_QUIET_SECS, None),
            Some(Some(secs)) if secs > 0 => (secs.unsigned_abs(), None),
            Some(_) => (
                WORKER_QUIET_SECS,
                Some(format!(
                    "[chat] worker_quiet_secs must be a positive integer; using {WORKER_QUIET_SECS}"
                )),
            ),
        }
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
