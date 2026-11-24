use crate::config::CodexSection;
use crate::workers::common_dir;
use ralph_core::worker::{Sandbox, SandboxMode};
use std::path::Path;

pub(super) fn configured_sandbox(codex: &CodexSection, cwd: &Path) -> Result<Sandbox, String> {
    let mode = codex.sandbox_mode()?;
    let writable_roots = match mode {
        Some(SandboxMode::WorkspaceWrite) => common_dir(cwd).into_iter().collect(),
        _ => Vec::new(),
    };
    Ok(Sandbox {
        mode,
        writable_roots,
    })
}
