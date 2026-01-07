use thiserror::Error;

pub type Result<T> = std::result::Result<T, LintError>;

#[derive(Error, Debug)]
pub enum LintError {
    #[error("Code quality checks failed")]
    ChecksFailed,

    #[error("Git hooks installation failed: {0}")]
    HooksInstallFailed(String),

    #[error("Command not found: {command}\n{help}")]
    CommandNotFound { command: String, help: String },

    #[error("Not a git repository")]
    NotGitRepository,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Check if a required command exists in PATH
pub fn require_command(command: &str, help: &str) -> Result<()> {
    if !command_exists(command) {
        return Err(LintError::CommandNotFound {
            command: command.to_string(),
            help: help.to_string(),
        });
    }
    Ok(())
}

/// Check if a command exists in the system PATH
fn command_exists(command: &str) -> bool {
    std::process::Command::new("which")
        .arg(command)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}
