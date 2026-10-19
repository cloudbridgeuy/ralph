use crate::config::AppConfig;
use crate::snapshot::Snapshotter;
use ralph_core::changes::{
    ref_prefix, resolve_range, ChangeFormat, ChangeRequest, ScopeQuery, SnapshotRange,
};
use ralph_core::snapshot::{snapshots_enabled, SnapshotBase, CHAT_OWNER};
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

fn resolve(
    store: &Path,
    cwd: &Path,
    request: &ChangeRequest<ScopeQuery>,
) -> Result<(Snapshotter, ChangeRequest, SnapshotRange), String> {
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
    Ok((snapshotter, resolved, range))
}

fn to_base(snapshotter: &Snapshotter, range: &SnapshotRange) -> Option<SnapshotBase> {
    range
        .to
        .as_deref()
        .and_then(|to| snapshotter.snapshot_base(to))
}

pub(super) fn run_changes(
    store: &Path,
    cwd: &Path,
    request: &ChangeRequest<ScopeQuery>,
) -> Result<String, String> {
    let (snapshotter, resolved, range) = resolve(store, cwd, request)?;
    let base = to_base(&snapshotter, &range);
    snapshotter
        .render_diff(&resolved, &range, base)
        .map_err(|e| format!("Could not diff snapshots: {e}"))
}

/// Resolve a chat or worker scope to a rebase-adjusted snapshot range for `/diff`.
pub(super) fn resolve_diff(
    store: &Path,
    cwd: &Path,
    scope: ScopeQuery,
) -> Result<(Snapshotter, SnapshotRange, Option<&'static str>), String> {
    let request = ChangeRequest {
        scope,
        format: ChangeFormat::Stat,
        paths: Vec::new(),
        max_bytes: None,
    };
    let (snapshotter, _, range) = resolve(store, cwd, &request)?;
    let base = to_base(&snapshotter, &range);
    let (range, note) = snapshotter.adjust_range(&range, base);
    Ok((snapshotter, range, note))
}
