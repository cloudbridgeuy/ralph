use super::{AppConfig, ConfigError};
use ralph_core::chat::resolve_watch_timeout;
use ralph_core::chat_cell::resolve_cell_limits;
use ralph_core::chat_edit_mode::resolve_edit_mode;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct CheckReport {
    pub errors: Vec<String>,
    pub notes: Vec<String>,
}

pub fn hard_errors(config: &AppConfig) -> Vec<String> {
    let chat = &config.chat;
    resolve_watch_timeout(chat.watch_timeout_secs)
        .err()
        .into_iter()
        .chain(
            resolve_cell_limits(None, None, chat.cell_output_limit, chat.cell_output_bytes).err(),
        )
        .collect()
}

pub fn check(content: &str, path: &str) -> Result<CheckReport, ConfigError> {
    let (config, unknown) = AppConfig::parse_reporting_unknown(content, path)?;
    let chat = &config.chat;
    let errors = unknown
        .into_iter()
        .map(|key| format!("unknown key: {key}"))
        .chain(config.theme.warnings())
        .chain(config.changes.warnings())
        .chain(chat.worker_quiet_setting().1)
        .chain(chat.reset_limits_setting().1)
        .chain(chat.empty_submit_setting().1)
        .chain(chat.editor_setting().1)
        .chain(resolve_edit_mode(chat.edit_mode.as_deref()).1)
        .chain(hard_errors(&config))
        .collect();
    Ok(CheckReport {
        errors,
        notes: config.theme.deprecations(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn errors(content: &str) -> Vec<String> {
        check(content, "config.toml").unwrap().errors
    }

    #[test]
    fn valid_file_has_no_errors() {
        assert_eq!(
            check("[chat]\nedit_mode = \"vi\"\n", "c").unwrap(),
            CheckReport::default()
        );
    }

    #[test]
    fn claude_shell_accepts_bash_and_rejects_unknown() {
        assert!(check("[claude]\nshell = \"fish\"\n", "c").is_err());
        assert!(check("[claude]\nshell = \"bash\"\n", "c")
            .unwrap()
            .errors
            .is_empty());
    }

    #[test]
    fn bad_toml_and_invalid_workers_fail() {
        assert!(check("[chat", "c").is_err());
        assert!(check("[workers]\nview = \"grid\"\n", "c").is_err());
    }

    #[test]
    fn unknown_keys_report_dotted_paths() {
        assert_eq!(
            errors("[chat]\nwrap_widh = 80\n"),
            ["unknown key: chat.wrap_widh"]
        );
        assert_eq!(
            errors("[wire.prices]\n[workers.x]\ny = 1\n"),
            ["unknown key: workers.x"]
        );
    }

    #[test]
    fn warning_only_values_are_errors() {
        assert_eq!(errors("[chat]\nedit_mode = \"nano\"\n").len(), 1);
        assert_eq!(errors("[theme.agent]\nbad = \"x\"\n").len(), 1);
    }

    #[test]
    fn zero_watch_timeout_is_an_error() {
        assert_eq!(
            errors("[chat]\nwatch_timeout_secs = 0\n"),
            ["watch_timeout_secs must be above 0."]
        );
    }

    #[test]
    fn legacy_agent_is_a_note_only() {
        let report = check("[theme]\nagent = \"x\"\n", "c").unwrap();
        assert!(report.errors.is_empty());
        assert_eq!(report.notes.len(), 1);
    }
}
