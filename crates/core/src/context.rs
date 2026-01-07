//! Context file path resolution and validation.
//!
//! This module provides pure functions for resolving context file paths
//! and determining which files need to be created. Following the Functional
//! Core pattern, all functions operate on data provided as arguments - no file I/O.

use std::path::{Path, PathBuf};

/// Default paths for context files relative to project root.
pub mod defaults {
    /// Default path for the design document.
    pub const DESIGN_FILE: &str = ".claude/designs/design.md";
    /// Default path for the PRD file.
    pub const PRD_FILE: &str = ".claude/plans/prd.toml";
    /// Default path for the progress file.
    pub const PROGRESS_FILE: &str = ".claude/plans/progress.txt";
}

/// Resolved paths for all context files.
#[derive(Debug, Clone)]
pub struct ContextPaths {
    /// Path to the design document.
    pub design: PathBuf,
    /// Path to the PRD file.
    pub prd: PathBuf,
    /// Path to the progress file.
    pub progress: PathBuf,
}

impl ContextPaths {
    /// Create context paths with defaults, applying any overrides.
    ///
    /// # Arguments
    ///
    /// * `project_root` - Base directory for resolving relative paths
    /// * `design_override` - Optional override for design file path
    /// * `prd_override` - Optional override for PRD file path
    /// * `progress_override` - Optional override for progress file path
    pub fn new(
        project_root: &Path,
        design_override: Option<&Path>,
        prd_override: Option<&Path>,
        progress_override: Option<&Path>,
    ) -> Self {
        Self {
            design: design_override
                .map(PathBuf::from)
                .unwrap_or_else(|| project_root.join(defaults::DESIGN_FILE)),
            prd: prd_override
                .map(PathBuf::from)
                .unwrap_or_else(|| project_root.join(defaults::PRD_FILE)),
            progress: progress_override
                .map(PathBuf::from)
                .unwrap_or_else(|| project_root.join(defaults::PROGRESS_FILE)),
        }
    }
}

/// Result of checking which context files need to be touched.
#[derive(Debug, Clone)]
pub struct ContextFilesTouch {
    /// Design file path if it needs to be created (doesn't exist).
    pub design: Option<PathBuf>,
    /// Progress file path if it needs to be created (doesn't exist).
    pub progress: Option<PathBuf>,
}

impl ContextFilesTouch {
    /// Returns true if any files need to be touched.
    pub fn has_files_to_create(&self) -> bool {
        self.design.is_some() || self.progress.is_some()
    }

    /// Returns an iterator over all files that need to be created.
    pub fn files_to_create(&self) -> impl Iterator<Item = &PathBuf> {
        self.design.iter().chain(self.progress.iter())
    }
}

/// Determine which context files need to be touched based on existence flags.
///
/// This is a pure function - it takes existence information and returns
/// which files need to be created. Actual file system checks happen at
/// the shell layer.
///
/// # Arguments
///
/// * `paths` - The resolved context file paths
/// * `design_exists` - Whether the design file already exists
/// * `progress_exists` - Whether the progress file already exists
///
/// # Returns
///
/// A struct indicating which files need to be touched (created).
///
/// # Note
///
/// The PRD is never touched - if it doesn't exist, initialization should
/// fail with an error. That check happens separately.
pub fn determine_files_to_touch(
    paths: &ContextPaths,
    design_exists: bool,
    progress_exists: bool,
) -> ContextFilesTouch {
    ContextFilesTouch {
        design: if design_exists {
            None
        } else {
            Some(paths.design.clone())
        },
        progress: if progress_exists {
            None
        } else {
            Some(paths.progress.clone())
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_paths_with_defaults() {
        let root = Path::new("/project");
        let paths = ContextPaths::new(root, None, None, None);

        assert_eq!(
            paths.design,
            PathBuf::from("/project/.claude/designs/design.md")
        );
        assert_eq!(paths.prd, PathBuf::from("/project/.claude/plans/prd.toml"));
        assert_eq!(
            paths.progress,
            PathBuf::from("/project/.claude/plans/progress.txt")
        );
    }

    #[test]
    fn test_context_paths_with_overrides() {
        let root = Path::new("/project");
        let paths = ContextPaths::new(
            root,
            Some(Path::new("/custom/design.md")),
            Some(Path::new("/custom/prd.toml")),
            Some(Path::new("/custom/progress.txt")),
        );

        assert_eq!(paths.design, PathBuf::from("/custom/design.md"));
        assert_eq!(paths.prd, PathBuf::from("/custom/prd.toml"));
        assert_eq!(paths.progress, PathBuf::from("/custom/progress.txt"));
    }

    #[test]
    fn test_context_paths_with_partial_overrides() {
        let root = Path::new("/project");
        let paths = ContextPaths::new(root, Some(Path::new("/custom/design.md")), None, None);

        assert_eq!(paths.design, PathBuf::from("/custom/design.md"));
        assert_eq!(paths.prd, PathBuf::from("/project/.claude/plans/prd.toml"));
        assert_eq!(
            paths.progress,
            PathBuf::from("/project/.claude/plans/progress.txt")
        );
    }

    #[test]
    fn test_determine_files_to_touch_none_exist() {
        let paths = ContextPaths {
            design: PathBuf::from("/project/.claude/designs/design.md"),
            prd: PathBuf::from("/project/.claude/plans/prd.toml"),
            progress: PathBuf::from("/project/.claude/plans/progress.txt"),
        };

        let to_touch = determine_files_to_touch(&paths, false, false);

        assert!(to_touch.has_files_to_create());
        assert_eq!(
            to_touch.design,
            Some(PathBuf::from("/project/.claude/designs/design.md"))
        );
        assert_eq!(
            to_touch.progress,
            Some(PathBuf::from("/project/.claude/plans/progress.txt"))
        );
    }

    #[test]
    fn test_determine_files_to_touch_all_exist() {
        let paths = ContextPaths {
            design: PathBuf::from("/project/.claude/designs/design.md"),
            prd: PathBuf::from("/project/.claude/plans/prd.toml"),
            progress: PathBuf::from("/project/.claude/plans/progress.txt"),
        };

        let to_touch = determine_files_to_touch(&paths, true, true);

        assert!(!to_touch.has_files_to_create());
        assert_eq!(to_touch.design, None);
        assert_eq!(to_touch.progress, None);
    }

    #[test]
    fn test_determine_files_to_touch_partial() {
        let paths = ContextPaths {
            design: PathBuf::from("/project/.claude/designs/design.md"),
            prd: PathBuf::from("/project/.claude/plans/prd.toml"),
            progress: PathBuf::from("/project/.claude/plans/progress.txt"),
        };

        // Only design exists
        let to_touch = determine_files_to_touch(&paths, true, false);
        assert!(to_touch.has_files_to_create());
        assert_eq!(to_touch.design, None);
        assert_eq!(
            to_touch.progress,
            Some(PathBuf::from("/project/.claude/plans/progress.txt"))
        );

        // Only progress exists
        let to_touch = determine_files_to_touch(&paths, false, true);
        assert!(to_touch.has_files_to_create());
        assert_eq!(
            to_touch.design,
            Some(PathBuf::from("/project/.claude/designs/design.md"))
        );
        assert_eq!(to_touch.progress, None);
    }

    #[test]
    fn test_files_to_create_iterator() {
        let paths = ContextPaths {
            design: PathBuf::from("/design.md"),
            prd: PathBuf::from("/prd.toml"),
            progress: PathBuf::from("/progress.txt"),
        };

        let to_touch = determine_files_to_touch(&paths, false, false);
        let files: Vec<_> = to_touch.files_to_create().collect();

        assert_eq!(files.len(), 2);
        assert!(files.contains(&&PathBuf::from("/design.md")));
        assert!(files.contains(&&PathBuf::from("/progress.txt")));
    }
}
