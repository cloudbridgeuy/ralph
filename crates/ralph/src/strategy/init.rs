//! Strategy sync — scaffolds default agent files, strategy files,
//! and `strategy.toml`.
//!
//! Follows FC-IS: `plan_file_actions()` is a pure function that decides what
//! actions to take based on file existence. `execute_sync()` is the imperative
//! shell that performs file I/O and prints output.

use std::path::{Path, PathBuf};

use super::assets;

/// Describes the action taken for a single file during sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitAction {
    /// File was created successfully.
    Created { path: PathBuf },
    /// File already existed and was skipped.
    Skipped { path: PathBuf },
    /// File existed and was overwritten with the latest version.
    Overwritten { path: PathBuf },
}

/// An error during strategy sync.
#[derive(Debug, thiserror::Error)]
pub enum InitError {
    /// I/O error during file creation.
    #[error("Failed to write {path}: {source}")]
    WriteFile {
        path: String,
        source: std::io::Error,
    },

    /// I/O error creating directories.
    #[error("Failed to create directory {path}: {source}")]
    CreateDir {
        path: String,
        source: std::io::Error,
    },
}

/// Plan which files to create or skip for a set of assets.
///
/// Pure function — takes an asset list, target directory, and existence flags.
/// Does NOT perform I/O.
fn plan_file_actions(
    target_dir: &Path,
    asset_files: &[(&str, &str)],
    existing_files: &[bool],
) -> Vec<InitAction> {
    debug_assert_eq!(asset_files.len(), existing_files.len());
    asset_files
        .iter()
        .zip(existing_files.iter())
        .map(|((filename, _content), &exists)| {
            let path = target_dir.join(filename);
            if exists {
                InitAction::Skipped { path }
            } else {
                InitAction::Created { path }
            }
        })
        .collect()
}

/// Decide whether strategy.toml should be created or overwritten.
///
/// Pure function — no I/O.
fn plan_strategy_toml_action(path: &Path, exists: bool) -> InitAction {
    let path = path.to_path_buf();
    if exists {
        InitAction::Overwritten { path }
    } else {
        InitAction::Created { path }
    }
}

/// Format a single `InitAction` as a summary line.
///
/// Pure function — no I/O.
fn format_action(action: &InitAction) -> String {
    match action {
        InitAction::Created { path } => format!("  Created: {}", path.display()),
        InitAction::Skipped { path } => {
            format!("  Skipped: {} (already exists)", path.display())
        }
        InitAction::Overwritten { path } => {
            format!("  Overwritten: {} (updated to latest)", path.display())
        }
    }
}

/// Format the sync summary for display.
///
/// Pure function — no I/O.
fn format_sync_summary(
    agent_actions: &[InitAction],
    strategy_actions: &[InitAction],
    strategy_toml_action: &InitAction,
) -> String {
    use std::fmt::Write;

    let mut out = String::new();
    let _ = writeln!(out, "Strategy synced:\n");
    let _ = writeln!(out, "{}", format_action(strategy_toml_action));

    for action in agent_actions.iter().chain(strategy_actions) {
        let _ = writeln!(out, "{}", format_action(action));
    }

    out
}

/// Check which files from an asset list already exist in the target directory.
///
/// Gathers existence flags for the pure planner. Performs I/O.
fn check_existing(target_dir: &Path, asset_files: &[(&str, &str)]) -> Vec<bool> {
    asset_files
        .iter()
        .map(|(filename, _)| target_dir.join(filename).exists())
        .collect()
}

/// Write files for `Created` actions from planned actions zipped with assets.
///
/// Only writes when the action is `Created`; all other variants are skipped.
/// Performs I/O.
fn write_planned_files(
    actions: &[InitAction],
    asset_files: &[(&str, &str)],
) -> Result<(), InitError> {
    debug_assert_eq!(actions.len(), asset_files.len());
    for (action, (_filename, content)) in actions.iter().zip(asset_files.iter()) {
        if let InitAction::Created { path } = action {
            std::fs::write(path, content).map_err(|source| InitError::WriteFile {
                path: path.display().to_string(),
                source,
            })?;
        }
    }
    Ok(())
}

/// Create a directory, mapping I/O errors to `InitError::CreateDir`.
fn create_dir(dir: &Path) -> Result<(), InitError> {
    std::fs::create_dir_all(dir).map_err(|source| InitError::CreateDir {
        path: dir.display().to_string(),
        source,
    })
}

/// Execute strategy sync.
///
/// Imperative shell — performs file I/O.
///
/// 1. Create `.claude/agents/` and `.claude/strategies/` directories
/// 2. Gather file-existence flags and plan actions (pure)
/// 3. Write agent files, strategy files, and `strategy.toml`
/// 4. Print summary
pub fn execute_sync(project_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let claude_dir = project_path.join(".claude");
    let agents_dir = claude_dir.join("agents");
    let strategies_dir = claude_dir.join("strategies");
    let strategy_path = claude_dir.join("strategy.toml");

    // 1. Create directories
    create_dir(&agents_dir)?;
    create_dir(&strategies_dir)?;

    // 2. Plan actions (pure)
    let agent_actions = plan_file_actions(
        &agents_dir,
        assets::AGENT_ASSETS,
        &check_existing(&agents_dir, assets::AGENT_ASSETS),
    );
    let strategy_actions = plan_file_actions(
        &strategies_dir,
        assets::STRATEGY_ASSETS,
        &check_existing(&strategies_dir, assets::STRATEGY_ASSETS),
    );
    let strategy_toml_action = plan_strategy_toml_action(&strategy_path, strategy_path.exists());

    // 3. Write files
    write_planned_files(&agent_actions, assets::AGENT_ASSETS)?;
    write_planned_files(&strategy_actions, assets::STRATEGY_ASSETS)?;

    // Always overwrite strategy.toml to keep it in sync with the bundled
    // version, unlike agent/strategy files which are user-owned.
    std::fs::write(&strategy_path, assets::STRATEGY_TOML).map_err(|source| {
        InitError::WriteFile {
            path: strategy_path.display().to_string(),
            source,
        }
    })?;

    // 4. Print summary
    let summary = format_sync_summary(&agent_actions, &strategy_actions, &strategy_toml_action);
    print!("{summary}");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // plan_file_actions tests (pure) — exercised with agent assets
    // =========================================================================

    #[test]
    fn test_plan_all_new() {
        let dir = Path::new("/project/.claude/agents");
        let existing = vec![false; assets::AGENT_ASSETS.len()];
        let actions = plan_file_actions(dir, assets::AGENT_ASSETS, &existing);

        assert_eq!(actions.len(), assets::AGENT_ASSETS.len());
        for action in &actions {
            assert!(matches!(action, InitAction::Created { .. }));
        }
    }

    #[test]
    fn test_plan_all_existing() {
        let dir = Path::new("/project/.claude/agents");
        let existing = vec![true; assets::AGENT_ASSETS.len()];
        let actions = plan_file_actions(dir, assets::AGENT_ASSETS, &existing);

        assert_eq!(actions.len(), assets::AGENT_ASSETS.len());
        for action in &actions {
            assert!(matches!(action, InitAction::Skipped { .. }));
        }
    }

    #[test]
    fn test_plan_mixed() {
        let dir = Path::new("/project/.claude/agents");
        let mut existing = vec![false; assets::AGENT_ASSETS.len()];
        existing[0] = true;
        let actions = plan_file_actions(dir, assets::AGENT_ASSETS, &existing);

        assert!(matches!(actions[0], InitAction::Skipped { .. }));
        for action in &actions[1..] {
            assert!(matches!(action, InitAction::Created { .. }));
        }
    }

    #[test]
    fn test_plan_paths_include_filename() {
        let dir = Path::new("/project/.claude/agents");
        let existing = vec![false; assets::AGENT_ASSETS.len()];
        let actions = plan_file_actions(dir, assets::AGENT_ASSETS, &existing);

        if let InitAction::Created { path } = &actions[0] {
            assert_eq!(path, &dir.join("architect.md"));
        } else {
            panic!("Expected Created action");
        }
    }

    // =========================================================================
    // plan_file_actions tests — exercised with strategy assets
    // =========================================================================

    #[test]
    fn test_plan_strategy_all_new() {
        let dir = Path::new("/project/.claude/strategies");
        let existing = vec![false; assets::STRATEGY_ASSETS.len()];
        let actions = plan_file_actions(dir, assets::STRATEGY_ASSETS, &existing);

        assert_eq!(actions.len(), assets::STRATEGY_ASSETS.len());
        for action in &actions {
            assert!(matches!(action, InitAction::Created { .. }));
        }
    }

    #[test]
    fn test_plan_strategy_all_existing() {
        let dir = Path::new("/project/.claude/strategies");
        let existing = vec![true; assets::STRATEGY_ASSETS.len()];
        let actions = plan_file_actions(dir, assets::STRATEGY_ASSETS, &existing);

        assert_eq!(actions.len(), assets::STRATEGY_ASSETS.len());
        for action in &actions {
            assert!(matches!(action, InitAction::Skipped { .. }));
        }
    }

    #[test]
    fn test_plan_strategy_paths_include_filename() {
        let dir = Path::new("/project/.claude/strategies");
        let existing = vec![false; assets::STRATEGY_ASSETS.len()];
        let actions = plan_file_actions(dir, assets::STRATEGY_ASSETS, &existing);

        if let InitAction::Created { path } = &actions[0] {
            assert_eq!(path, &dir.join("prd-loop.toml"));
        } else {
            panic!("Expected Created action");
        }
    }

    // =========================================================================
    // format_action tests (pure)
    // =========================================================================

    #[test]
    fn test_format_overwritten_action() {
        let action = InitAction::Overwritten {
            path: PathBuf::from(".claude/strategy.toml"),
        };
        let output = format_action(&action);
        assert_eq!(
            output,
            "  Overwritten: .claude/strategy.toml (updated to latest)"
        );
    }

    // =========================================================================
    // plan_strategy_toml_action tests (pure)
    // =========================================================================

    #[test]
    fn test_plan_strategy_toml_new() {
        let path = Path::new(".claude/strategy.toml");
        let action = plan_strategy_toml_action(path, false);
        assert_eq!(
            action,
            InitAction::Created {
                path: path.to_path_buf()
            }
        );
    }

    #[test]
    fn test_plan_strategy_toml_existing() {
        let path = Path::new(".claude/strategy.toml");
        let action = plan_strategy_toml_action(path, true);
        assert_eq!(
            action,
            InitAction::Overwritten {
                path: path.to_path_buf()
            }
        );
    }

    // =========================================================================
    // format_sync_summary tests (pure)
    // =========================================================================

    #[test]
    fn test_format_summary_all_created() {
        let agent_actions = vec![
            InitAction::Created {
                path: PathBuf::from(".claude/agents/architect.md"),
            },
            InitAction::Created {
                path: PathBuf::from(".claude/agents/developer.md"),
            },
        ];
        let strategy_toml_action = InitAction::Created {
            path: PathBuf::from(".claude/strategy.toml"),
        };
        let output = format_sync_summary(&agent_actions, &[], &strategy_toml_action);
        assert!(output.contains("Created: .claude/strategy.toml"));
        assert!(output.contains("Created: .claude/agents/architect.md"));
        assert!(output.contains("Created: .claude/agents/developer.md"));
        assert!(!output.contains("Skipped"));
    }

    #[test]
    fn test_format_summary_with_skipped() {
        let agent_actions = vec![
            InitAction::Skipped {
                path: PathBuf::from(".claude/agents/architect.md"),
            },
            InitAction::Created {
                path: PathBuf::from(".claude/agents/developer.md"),
            },
        ];
        let strategy_toml_action = InitAction::Created {
            path: PathBuf::from(".claude/strategy.toml"),
        };
        let output = format_sync_summary(&agent_actions, &[], &strategy_toml_action);
        assert!(output.contains("Skipped: .claude/agents/architect.md (already exists)"));
        assert!(output.contains("Created: .claude/agents/developer.md"));
    }

    #[test]
    fn test_format_summary_includes_strategy_actions() {
        let agent_actions = vec![InitAction::Created {
            path: PathBuf::from(".claude/agents/developer.md"),
        }];
        let strategy_actions = vec![InitAction::Created {
            path: PathBuf::from(".claude/strategies/prd-loop.toml"),
        }];
        let strategy_toml_action = InitAction::Created {
            path: PathBuf::from(".claude/strategy.toml"),
        };
        let output = format_sync_summary(&agent_actions, &strategy_actions, &strategy_toml_action);
        assert!(output.contains("Created: .claude/agents/developer.md"));
        assert!(output.contains("Created: .claude/strategies/prd-loop.toml"));
    }

    #[test]
    fn test_format_summary_strategy_skipped() {
        let strategy_actions = vec![InitAction::Skipped {
            path: PathBuf::from(".claude/strategies/prd-loop.toml"),
        }];
        let strategy_toml_action = InitAction::Created {
            path: PathBuf::from(".claude/strategy.toml"),
        };
        let output = format_sync_summary(&[], &strategy_actions, &strategy_toml_action);
        assert!(output.contains("Skipped: .claude/strategies/prd-loop.toml (already exists)"));
    }

    #[test]
    fn test_format_summary_with_overwritten_strategy_toml() {
        let agent_actions = vec![InitAction::Created {
            path: PathBuf::from(".claude/agents/storyteller.md"),
        }];
        let strategy_toml_action = InitAction::Overwritten {
            path: PathBuf::from(".claude/strategy.toml"),
        };
        let output = format_sync_summary(&agent_actions, &[], &strategy_toml_action);
        assert!(output.contains("Overwritten: .claude/strategy.toml (updated to latest)"));
        assert!(output.contains("Created: .claude/agents/storyteller.md"));
    }

    // =========================================================================
    // Bundled assets sanity tests
    // =========================================================================

    #[test]
    #[allow(clippy::const_is_empty)]
    fn test_bundled_assets_not_empty() {
        for (filename, content) in assets::AGENT_ASSETS {
            assert!(!content.is_empty(), "{filename} should not be empty");
        }
        assert!(!assets::STRATEGY_TOML.is_empty());
    }

    #[test]
    fn test_bundled_agents_have_frontmatter() {
        for (filename, content) in assets::AGENT_ASSETS {
            assert!(
                content.starts_with("---\n"),
                "{filename} should start with YAML frontmatter delimiter"
            );
            assert!(
                content.contains("name:"),
                "{filename} should have a name field"
            );
            assert!(
                content.contains("description:"),
                "{filename} should have a description field"
            );
        }
    }

    #[test]
    fn test_bundled_strategy_toml_has_agents_table() {
        assert!(assets::STRATEGY_TOML.contains("[agents]"));
    }

    #[test]
    fn test_agent_assets_count() {
        assert_eq!(assets::AGENT_ASSETS.len(), 9);
    }

    // =========================================================================
    // Bundled strategy assets sanity tests
    // =========================================================================

    #[test]
    #[allow(clippy::const_is_empty)]
    fn test_bundled_strategy_assets_not_empty() {
        for (filename, content) in assets::STRATEGY_ASSETS {
            assert!(!content.is_empty(), "{filename} should not be empty");
        }
    }

    #[test]
    fn test_bundled_strategy_assets_are_valid_toml() {
        for (filename, content) in assets::STRATEGY_ASSETS {
            let result = ralph_core::strategy::parse_strategy(content, filename);
            assert!(
                result.is_ok(),
                "{filename} should be valid strategy TOML: {}",
                result.unwrap_err()
            );
        }
    }

    #[test]
    fn test_strategy_assets_count() {
        assert_eq!(assets::STRATEGY_ASSETS.len(), 2);
    }
}
