use crate::config::AppConfig;
use crate::snapshot::Snapshotter;
use crate::worker_changes::run_diff;
use ralph_core::changes::{
    diff_args, ref_prefix, render_changes, resolve_range, ChangeRequest, ScopeQuery,
};
use ralph_core::snapshot::{snapshots_enabled, CHAT_OWNER};
use std::path::Path;

const SNAPSHOTS_OFF: &str = "snapshots are off ([changes] enabled = false)";
const NO_SNAPSHOTS: &str = "no snapshots";

fn changes_enabled() -> Result<(), String> {
    let config = AppConfig::load().map_err(|e| format!("Could not load config: {e}"))?;
    if snapshots_enabled(Some(config.changes.enabled)) {
        Ok(())
    } else {
        Err(SNAPSHOTS_OFF.to_string())
    }
}

pub(super) fn run_changes(
    store: &Path,
    cwd: &Path,
    request: &ChangeRequest<ScopeQuery>,
) -> Result<String, String> {
    changes_enabled()?;
    let snapshotter =
        Snapshotter::open(store, cwd, CHAT_OWNER).ok_or_else(|| NO_SNAPSHOTS.to_string())?;
    let refs = snapshotter
        .refs(&ref_prefix(&request.scope))
        .map_err(|e| format!("Could not list snapshots: {e}"))?;
    if refs.trim().is_empty() {
        return Err(NO_SNAPSHOTS.to_string());
    }
    let (resolved, range) = resolve_range(request, &refs)?;
    let output = run_diff(&diff_args(&resolved, &range), |command| {
        command
            .env("GIT_DIR", snapshotter.side())
            .current_dir(snapshotter.side());
    })
    .map_err(|e| format!("Could not diff snapshots: {e}"))?;
    Ok(render_changes(&resolved, &output))
}
