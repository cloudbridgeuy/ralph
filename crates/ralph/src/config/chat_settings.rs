use super::empty_submit;
use super::{ChatSection, EditorSetting, WORKER_QUIET_SECS};
use ralph_core::chat::{ResetLimits, RESET_TOKENS, RESET_TURNS};
use ralph_core::chat_send_it::Limits;
use ralph_core::chat_skills::expand_home;
use ralph_core::chat_status::EmptySubmit;
use std::path::{Path, PathBuf};

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

pub const SEND_IT_DEFAULT_MAX_USD: f64 = 5.0;
pub const SEND_IT_DEFAULT_MAX_TURNS: u32 = 15;

pub fn clamp_send_it(max_turns: u32, max_usd: f64) -> (Limits, Vec<String>) {
    let mut warnings = Vec::new();
    let max_turns = if max_turns == 0 {
        warnings.push(format!(
            "[chat] send_it_max_turns must be a positive integer; using {SEND_IT_DEFAULT_MAX_TURNS}"
        ));
        SEND_IT_DEFAULT_MAX_TURNS
    } else {
        max_turns
    };
    let max_usd = if max_usd.is_finite() && max_usd > 0.0 {
        max_usd
    } else {
        warnings.push(format!(
            "[chat] send_it_max_usd must be a positive number; using {SEND_IT_DEFAULT_MAX_USD}"
        ));
        SEND_IT_DEFAULT_MAX_USD
    };
    (Limits { max_turns, max_usd }, warnings)
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct PromptFiles {
    pub behaviour: Option<PathBuf>,
    pub appends: Vec<PathBuf>,
}

impl ChatSection {
    pub fn prompt_files(&self, home: Option<&Path>) -> PromptFiles {
        PromptFiles {
            behaviour: self
                .prompt_file
                .as_deref()
                .map(|path| expand_home(path, home)),
            appends: self
                .append_prompt_files
                .iter()
                .map(|path| expand_home(path, home))
                .collect(),
        }
    }

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

    pub fn send_it_setting(&self) -> (Limits, Vec<String>) {
        clamp_send_it(self.send_it_max_turns, self.send_it_max_usd)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_files_expand_home_and_keep_other_paths() {
        let chat = ChatSection {
            prompt_file: Some(PathBuf::from("~/p.md")),
            append_prompt_files: vec![PathBuf::from("/cfg/a.md"), PathBuf::from("~")],
            ..ChatSection::default()
        };
        assert_eq!(
            chat.prompt_files(Some(Path::new("/h"))),
            PromptFiles {
                behaviour: Some(PathBuf::from("/h/p.md")),
                appends: vec![PathBuf::from("/cfg/a.md"), PathBuf::from("/h")],
            }
        );
        assert_eq!(
            chat.prompt_files(None).behaviour,
            Some(PathBuf::from("~/p.md"))
        );
    }

    #[test]
    fn send_it_values_in_range_pass_through_without_warnings() {
        assert_eq!(
            clamp_send_it(1, 0.01),
            (
                Limits {
                    max_turns: 1,
                    max_usd: 0.01
                },
                Vec::new()
            )
        );
        let (settings, warnings) = ChatSection::default().send_it_setting();
        assert_eq!(
            settings,
            Limits {
                max_turns: 15,
                max_usd: 5.0
            }
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn send_it_bad_max_usd_resets_to_five() {
        for usd in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let (settings, warnings) = clamp_send_it(15, usd);
            assert_eq!(settings.max_usd, 5.0, "{usd}");
            assert_eq!(warnings.len(), 1);
            assert!(warnings[0].contains("send_it_max_usd"));
        }
    }

    #[test]
    fn send_it_zero_max_turns_resets_to_fifteen() {
        let (settings, warnings) = clamp_send_it(0, 5.0);
        assert_eq!(settings.max_turns, 15);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("send_it_max_turns"));
    }

    #[test]
    fn send_it_each_bad_key_warns_separately() {
        let (settings, warnings) = clamp_send_it(0, 0.0);
        assert_eq!(
            settings,
            Limits {
                max_turns: 15,
                max_usd: 5.0
            }
        );
        assert_eq!(warnings.len(), 2);
    }
}
