use crate::snapshot::Snapshotter;
use crate::warn::{warn, warn_if_err};
use ralph_core::snapshot::{chat_ref, Edge, CHAT_OWNER};
use std::path::Path;

pub(super) struct TurnSnapshots(Option<Snapshotter>);

impl TurnSnapshots {
    pub(super) fn open(enabled: bool, store: &Path, cwd: &Path) -> Self {
        Self(
            enabled
                .then(|| Snapshotter::open(store, cwd, CHAT_OWNER))
                .flatten(),
        )
    }

    pub(super) fn around<T>(&self, run: impl FnOnce() -> T) -> T {
        let Some(snapshotter) = &self.0 else {
            return run();
        };
        let Ok(turn) = snapshotter
            .next_turn()
            .inspect_err(|e| warn(format!("Failed to number the turn snapshot: {e}")))
        else {
            return run();
        };
        warn_if_err(
            snapshotter.snapshot(&chat_ref(turn, Edge::Start)),
            "Failed to snapshot the turn start",
        );
        let result = run();
        warn_if_err(
            snapshotter.snapshot(&chat_ref(turn, Edge::End)),
            "Failed to snapshot the turn end",
        );
        warn_if_err(snapshotter.prune(), "Failed to prune turn snapshots");
        result
    }
}
