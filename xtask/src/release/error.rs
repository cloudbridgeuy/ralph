use ralph_core::semver::Version;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, ReleaseError>;

#[derive(Error, Debug)]
pub enum ReleaseError {
    #[error("Invalid version '{0}': expected x.y.z")]
    InvalidVersion(String),

    #[error("Version {new} must be greater than the current version {current}")]
    NotNewer { current: Version, new: Version },

    #[error("No version line found in [workspace.package] of Cargo.toml")]
    VersionLineNotFound,

    #[error("Working tree is not clean; commit or stash changes first")]
    DirtyWorkingTree,

    #[error("Tag {0} already exists")]
    TagExists(String),

    #[error("Command failed: {0}")]
    CommandFailed(String),

    #[error("Invalid Linear response: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Linear response has no data")]
    MissingData,

    #[error("Release notes are empty; aborting")]
    EmptyNotes,

    #[error("Linear request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
