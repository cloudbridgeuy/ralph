use crate::highlight::ThemeConfig;
use crate::stream_processor::VerboseToolsConfig;
use crate::turn_sink::TerminalSink;
use ralph_core::cancel::CancelToken;
use ralph_core::worker::ClaudeMode;
use ralph_server::invoke::{build_command, prompt_stdin};
use ralph_server::subprocess::{
    invoke_subprocess_with_spinner_config, SpinnerSubprocessConfig, StreamingSubprocessResult,
    SubprocessError,
};
use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct InvocationConfig {
    pub prompt: String,
    pub timeout_secs: u64,
    pub theme_config: ThemeConfig,
    pub verbose_tools: VerboseToolsConfig,
    pub permission_mode: ClaudeMode,
    pub config_dir: Option<PathBuf>,
    pub interrupt: CancelToken,
}

pub fn invoke(config: InvocationConfig) -> Result<StreamingSubprocessResult, SubprocessError> {
    let sink = Arc::new(
        TerminalSink::plain(&config.theme_config)
            .map_err(|error| SubprocessError::Theme(error.to_string()))?,
    );
    invoke_subprocess_with_spinner_config(&SpinnerSubprocessConfig {
        command: build_command(
            config.permission_mode,
            config.config_dir.as_deref(),
            dirs::home_dir().as_deref(),
        ),
        timeout_secs: Some(config.timeout_secs),
        verbose_tools: config.verbose_tools,
        show_tools: io::stdout().is_terminal(),
        terminated: crate::signal::is_terminated,
        sink,
        gate: None,
        interrupt: Some(config.interrupt),
        stdin: Some(prompt_stdin(&config.prompt)),
    })
}
