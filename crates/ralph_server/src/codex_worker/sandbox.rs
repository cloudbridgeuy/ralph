use crate::config::CodexSection;
use crate::workers::common_dir;
use ralph_core::worker::{scheduled_sandbox_mode, Sandbox, SandboxMode};
use std::path::Path;

pub(super) fn configured_sandbox(
    codex: &CodexSection,
    cwd: &Path,
    scheduled: bool,
) -> Result<Sandbox, String> {
    let mode = scheduled_sandbox_mode(scheduled, codex.sandbox_mode()?);
    let writable_roots = match mode {
        Some(SandboxMode::WorkspaceWrite) => common_dir(cwd).into_iter().collect(),
        _ => Vec::new(),
    };
    Ok(Sandbox {
        mode,
        writable_roots,
    })
}
