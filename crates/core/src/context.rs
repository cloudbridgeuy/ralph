//! Context file path resolution and validation.
//!
//! This module provides pure functions for resolving context file paths
//! and determining which files need to be created. Following the Functional
//! Core pattern, all functions operate on data provided as arguments - no file I/O.

use std::path::{Path, PathBuf};

/// Default paths and templates for context files and commands.
pub mod defaults {
    /// Default path for the design document.
    pub const DESIGN_FILE: &str = ".local/designs/design.md";
    /// Default path for the PRD file.
    pub const PRD_FILE: &str = ".local/plans/prd.toml";
    /// Default path for the progress file.
    pub const PROGRESS_FILE: &str = ".local/plans/progress.txt";

    /// Default command template for summarizing the progress file.
    ///
    /// Uses Claude CLI with:
    /// - `--dangerously-skip-permissions`: Bypass permission checks since summarization
    ///   only outputs to stdout (no file access needed by Claude itself)
    /// - `-p {prompt}`: Non-interactive print mode with prompt
    pub const SUMMARIZE_COMMAND: &str = "claude --dangerously-skip-permissions -p {prompt}";

    /// Default prompt template for summarizing the progress file.
    ///
    /// # Placeholders
    ///
    /// - `{progress_file}` - Path to the progress file
    /// - `{progress_content}` - Actual content of the progress file
    pub const SUMMARIZE_PROMPT: &str = r#"You are summarizing a progress notes file for an LLM-driven development workflow.

The file is located at: {progress_file}

Current content:
```
{progress_content}
```

Your task is to create a condensed summary that:
1. PRESERVES all key technical decisions and their rationale
2. PRESERVES all blockers, open questions, and unresolved issues
3. PRESERVES implementation details that would be needed to continue work
4. REMOVES redundant entries (e.g., multiple "starting feature X" entries)
5. REMOVES outdated information that has been superseded
6. CONSOLIDATES related entries into coherent summaries
7. MAINTAINS chronological context where it matters

Target length: 200-500 lines (significantly shorter than the original)

Output ONLY the summarized content, ready to replace the file. Do not include any preamble or explanation."#;

    /// Default command template for invoking the LLM.
    ///
    /// Uses Claude CLI with:
    /// - `--permission-mode acceptEdits`: Auto-accept file edits
    /// - `--output-format stream-json`: JSON streaming output for metadata extraction
    /// - `-p {prompt}`: Prompt placeholder to be substituted
    ///
    /// **Important**: The `--output-format stream-json` flag is required for ralph to extract
    /// session metadata (model, costs, usage) and tool interactions from the output.
    /// Custom commands can override this format, but metadata extraction will be unavailable.
    pub const COMMAND_TEMPLATE: &str =
        "claude --verbose --permission-mode acceptEdits --output-format stream-json -p {prompt}";

    /// Default completion marker string.
    ///
    /// When this marker appears in the LLM output, ralph exits the iteration loop
    /// regardless of PRD state. This allows the LLM to signal completion explicitly.
    pub const COMPLETION_MARKER: &str = "<promise>COMPLETE</promise>";

    /// Default prompt template for the LLM.
    ///
    /// This template is used when no `--prompt` argument is provided.
    /// It instructs the LLM to implement a single user story from the PRD.
    ///
    /// # Placeholders
    ///
    /// - `{design_file}` - Path to the design document
    /// - `{prd_file}` - Path to the PRD file
    /// - `{progress_file}` - Path to the progress notes file
    /// - `{completion_marker}` - The marker string to output when all stories are complete
    /// - `{additional_prompt}` - Additional instructions appended to the prompt (optional)
    ///
    /// # Notes
    ///
    /// The template uses `@` file references which are understood by Claude CLI
    /// to read and include the file contents in the context.
    pub const PROMPT_TEMPLATE: &str = r#"@{design_file} @{prd_file} @{progress_file}

1. Find the highest-priority feature to work on and work on that feature.
   This should be the one YOU decide has the highest priority - not necessarily the first in the list.

2. Check that the 'cargo xtask lint' command passes successfully.
   You can't mark a user story as complete if this command fails.
   Even when the issue is not related to your current changes.

3. Update the PRD with the work that was done by setting passes = true for completed stories.

4. Append your progress to the progress.txt file.
   Use this to leave a note for the next person working in the codebase.

5. Make a git commit of that feature without Claude attribution.

6. If you find some PRD is missing in order to complete or extend the task you are working on, you may append it to the PRD using the appropriate format.

ONLY WORK ON A SINGLE FEATURE.

IF YOU NOTICE A FILE GOING OVER 1000 LINES CONSIDER UPDATING IT INTO A MODULE OR MOVING THE TESTS TO A DIFFERENT FILE, AND USE THE #[path = ...] PATTERN.
{additional_prompt}
If, while implementing the feature, you notice all stories in the PRD are complete, output {completion_marker}."#;
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

/// Substitute placeholders in a prompt template.
///
/// This is a pure function that replaces known placeholders with their values.
/// Unknown placeholders are left unchanged.
///
/// # Placeholders
///
/// - `{design_file}` - Replaced with the design file path
/// - `{prd_file}` - Replaced with the PRD file path
/// - `{progress_file}` - Replaced with the progress file path
/// - `{completion_marker}` - Replaced with the completion marker string
/// - `{additional_prompt}` - Replaced with additional prompt instructions
///
/// # Arguments
///
/// * `template` - The prompt template containing placeholders
/// * `paths` - The context paths to substitute
/// * `completion_marker` - The completion marker string
/// * `additional_prompt` - Additional instructions to append
///
/// # Returns
///
/// The template with all known placeholders replaced.
///
/// # Example
///
/// ```
/// use ralph_core::context::{ContextPaths, substitute_template_placeholders};
/// use std::path::Path;
///
/// let paths = ContextPaths::new(Path::new("/project"), None, None, None);
/// let template = "Read @{design_file} and @{prd_file}";
/// let result = substitute_template_placeholders(template, &paths, "<promise>COMPLETE</promise>", "");
///
/// assert!(result.contains("/project/.local/designs/design.md"));
/// assert!(result.contains("/project/.local/plans/prd.toml"));
/// ```
pub fn substitute_template_placeholders(
    template: &str,
    paths: &ContextPaths,
    completion_marker: &str,
    additional_prompt: &str,
) -> String {
    template
        .replace("{design_file}", &paths.design.display().to_string())
        .replace("{prd_file}", &paths.prd.display().to_string())
        .replace("{progress_file}", &paths.progress.display().to_string())
        .replace("{completion_marker}", completion_marker)
        .replace("{additional_prompt}", additional_prompt)
}

/// Substitute placeholders in a summarization prompt template.
///
/// This is a pure function that replaces summarization-specific placeholders.
/// Unknown placeholders are left unchanged.
///
/// # Placeholders
///
/// - `{progress_file}` - Replaced with the progress file path
/// - `{progress_content}` - Replaced with the actual content of the progress file
///
/// # Arguments
///
/// * `template` - The prompt template containing placeholders
/// * `progress_path` - Path to the progress file
/// * `progress_content` - Content of the progress file
///
/// # Returns
///
/// The template with all known placeholders replaced.
pub fn substitute_summarize_placeholders(
    template: &str,
    progress_path: &str,
    progress_content: &str,
) -> String {
    template
        .replace("{progress_file}", progress_path)
        .replace("{progress_content}", progress_content)
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
            PathBuf::from("/project/.local/designs/design.md")
        );
        assert_eq!(paths.prd, PathBuf::from("/project/.local/plans/prd.toml"));
        assert_eq!(
            paths.progress,
            PathBuf::from("/project/.local/plans/progress.txt")
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
        assert_eq!(paths.prd, PathBuf::from("/project/.local/plans/prd.toml"));
        assert_eq!(
            paths.progress,
            PathBuf::from("/project/.local/plans/progress.txt")
        );
    }

    #[test]
    fn test_determine_files_to_touch_none_exist() {
        let paths = ContextPaths {
            design: PathBuf::from("/project/.local/designs/design.md"),
            prd: PathBuf::from("/project/.local/plans/prd.toml"),
            progress: PathBuf::from("/project/.local/plans/progress.txt"),
        };

        let to_touch = determine_files_to_touch(&paths, false, false);

        assert!(to_touch.has_files_to_create());
        assert_eq!(
            to_touch.design,
            Some(PathBuf::from("/project/.local/designs/design.md"))
        );
        assert_eq!(
            to_touch.progress,
            Some(PathBuf::from("/project/.local/plans/progress.txt"))
        );
    }

    #[test]
    fn test_determine_files_to_touch_all_exist() {
        let paths = ContextPaths {
            design: PathBuf::from("/project/.local/designs/design.md"),
            prd: PathBuf::from("/project/.local/plans/prd.toml"),
            progress: PathBuf::from("/project/.local/plans/progress.txt"),
        };

        let to_touch = determine_files_to_touch(&paths, true, true);

        assert!(!to_touch.has_files_to_create());
        assert_eq!(to_touch.design, None);
        assert_eq!(to_touch.progress, None);
    }

    #[test]
    fn test_determine_files_to_touch_partial() {
        let paths = ContextPaths {
            design: PathBuf::from("/project/.local/designs/design.md"),
            prd: PathBuf::from("/project/.local/plans/prd.toml"),
            progress: PathBuf::from("/project/.local/plans/progress.txt"),
        };

        // Only design exists
        let to_touch = determine_files_to_touch(&paths, true, false);
        assert!(to_touch.has_files_to_create());
        assert_eq!(to_touch.design, None);
        assert_eq!(
            to_touch.progress,
            Some(PathBuf::from("/project/.local/plans/progress.txt"))
        );

        // Only progress exists
        let to_touch = determine_files_to_touch(&paths, false, true);
        assert!(to_touch.has_files_to_create());
        assert_eq!(
            to_touch.design,
            Some(PathBuf::from("/project/.local/designs/design.md"))
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

    #[test]
    fn test_default_command_template_contains_stream_json() {
        assert!(defaults::COMMAND_TEMPLATE.contains("--output-format stream-json"));
    }

    #[test]
    fn test_default_command_template_contains_prompt_placeholder() {
        assert!(defaults::COMMAND_TEMPLATE.contains("{prompt}"));
    }

    #[test]
    fn test_default_command_template_contains_permission_mode() {
        assert!(defaults::COMMAND_TEMPLATE.contains("--permission-mode acceptEdits"));
    }

    #[test]
    fn test_default_command_template_uses_claude() {
        assert!(defaults::COMMAND_TEMPLATE.starts_with("claude "));
    }

    #[test]
    fn test_default_completion_marker() {
        assert_eq!(defaults::COMPLETION_MARKER, "<promise>COMPLETE</promise>");
    }

    // Tests for PROMPT_TEMPLATE constant

    #[test]
    fn test_default_prompt_template_contains_design_placeholder() {
        assert!(defaults::PROMPT_TEMPLATE.contains("{design_file}"));
    }

    #[test]
    fn test_default_prompt_template_contains_prd_placeholder() {
        assert!(defaults::PROMPT_TEMPLATE.contains("{prd_file}"));
    }

    #[test]
    fn test_default_prompt_template_contains_progress_placeholder() {
        assert!(defaults::PROMPT_TEMPLATE.contains("{progress_file}"));
    }

    #[test]
    fn test_default_prompt_template_contains_completion_marker_placeholder() {
        assert!(defaults::PROMPT_TEMPLATE.contains("{completion_marker}"));
    }

    #[test]
    fn test_default_prompt_template_uses_at_file_references() {
        // Claude CLI uses @ for file references
        assert!(defaults::PROMPT_TEMPLATE.contains("@{design_file}"));
        assert!(defaults::PROMPT_TEMPLATE.contains("@{prd_file}"));
        assert!(defaults::PROMPT_TEMPLATE.contains("@{progress_file}"));
    }

    #[test]
    fn test_default_prompt_template_includes_key_instructions() {
        // Key workflow instructions should be present
        assert!(defaults::PROMPT_TEMPLATE.contains("highest-priority feature"));
        assert!(defaults::PROMPT_TEMPLATE.contains("passes = true"));
        assert!(defaults::PROMPT_TEMPLATE.contains("ONLY WORK ON A SINGLE FEATURE"));
    }

    // Tests for substitute_template_placeholders function

    #[test]
    fn test_substitute_all_placeholders() {
        let paths = ContextPaths {
            design: PathBuf::from("/project/design.md"),
            prd: PathBuf::from("/project/prd.toml"),
            progress: PathBuf::from("/project/progress.txt"),
        };

        let template =
            "{design_file} {prd_file} {progress_file} {completion_marker} {additional_prompt}";
        let result = substitute_template_placeholders(template, &paths, "DONE", "extra");

        assert_eq!(
            result,
            "/project/design.md /project/prd.toml /project/progress.txt DONE extra"
        );
    }

    #[test]
    fn test_substitute_preserves_unknown_placeholders() {
        let paths = ContextPaths {
            design: PathBuf::from("/design.md"),
            prd: PathBuf::from("/prd.toml"),
            progress: PathBuf::from("/progress.txt"),
        };

        let template = "{design_file} {unknown_placeholder} {another}";
        let result = substitute_template_placeholders(template, &paths, "DONE", "");

        assert!(result.contains("/design.md"));
        assert!(result.contains("{unknown_placeholder}"));
        assert!(result.contains("{another}"));
    }

    #[test]
    fn test_substitute_with_default_prompt_template() {
        let paths = ContextPaths {
            design: PathBuf::from("/my/design.md"),
            prd: PathBuf::from("/my/prd.toml"),
            progress: PathBuf::from("/my/progress.txt"),
        };

        let result =
            substitute_template_placeholders(defaults::PROMPT_TEMPLATE, &paths, "COMPLETE", "");

        assert!(result.contains("@/my/design.md"));
        assert!(result.contains("@/my/prd.toml"));
        assert!(result.contains("@/my/progress.txt"));
        assert!(result.contains("output COMPLETE"));
        // No remaining placeholders for known values
        assert!(!result.contains("{design_file}"));
        assert!(!result.contains("{prd_file}"));
        assert!(!result.contains("{progress_file}"));
        assert!(!result.contains("{completion_marker}"));
    }

    #[test]
    fn test_substitute_empty_template() {
        let paths = ContextPaths {
            design: PathBuf::from("/design.md"),
            prd: PathBuf::from("/prd.toml"),
            progress: PathBuf::from("/progress.txt"),
        };

        let result = substitute_template_placeholders("", &paths, "DONE", "");
        assert_eq!(result, "");
    }

    #[test]
    fn test_substitute_no_placeholders() {
        let paths = ContextPaths {
            design: PathBuf::from("/design.md"),
            prd: PathBuf::from("/prd.toml"),
            progress: PathBuf::from("/progress.txt"),
        };

        let template = "No placeholders here";
        let result = substitute_template_placeholders(template, &paths, "DONE", "");
        assert_eq!(result, "No placeholders here");
    }

    #[test]
    fn test_substitute_multiple_occurrences() {
        let paths = ContextPaths {
            design: PathBuf::from("/design.md"),
            prd: PathBuf::from("/prd.toml"),
            progress: PathBuf::from("/progress.txt"),
        };

        let template = "{design_file} and also {design_file}";
        let result = substitute_template_placeholders(template, &paths, "DONE", "");
        assert_eq!(result, "/design.md and also /design.md");
    }

    #[test]
    fn test_substitute_with_special_characters_in_paths() {
        let paths = ContextPaths {
            design: PathBuf::from("/path with spaces/design.md"),
            prd: PathBuf::from("/special!@#$/prd.toml"),
            progress: PathBuf::from("/unicode/进度.txt"),
        };

        let template = "{design_file}|{prd_file}|{progress_file}";
        let result = substitute_template_placeholders(template, &paths, "DONE", "");

        assert!(result.contains("/path with spaces/design.md"));
        assert!(result.contains("/special!@#$/prd.toml"));
        assert!(result.contains("/unicode/进度.txt"));
    }

    #[test]
    fn test_substitute_additional_prompt() {
        let paths = ContextPaths {
            design: PathBuf::from("/design.md"),
            prd: PathBuf::from("/prd.toml"),
            progress: PathBuf::from("/progress.txt"),
        };

        let template = "Main instructions\n\n{additional_prompt}";
        let result =
            substitute_template_placeholders(template, &paths, "DONE", "Custom extra instructions");

        assert!(result.contains("Main instructions"));
        assert!(result.contains("Custom extra instructions"));
        assert!(!result.contains("{additional_prompt}"));
    }

    #[test]
    fn test_substitute_empty_additional_prompt() {
        let paths = ContextPaths {
            design: PathBuf::from("/design.md"),
            prd: PathBuf::from("/prd.toml"),
            progress: PathBuf::from("/progress.txt"),
        };

        let template = "Instructions{additional_prompt}";
        let result = substitute_template_placeholders(template, &paths, "DONE", "");

        assert_eq!(result, "Instructions");
    }

    // Tests for summarization defaults and functions

    #[test]
    fn test_default_summarize_command() {
        assert!(defaults::SUMMARIZE_COMMAND.contains("{prompt}"));
        assert!(defaults::SUMMARIZE_COMMAND.contains("claude"));
    }

    #[test]
    fn test_default_summarize_command_has_permission_bypass() {
        // Summarization runs non-interactively and needs permission bypass
        // to avoid interactive prompts blocking the subprocess
        assert!(defaults::SUMMARIZE_COMMAND.contains("--dangerously-skip-permissions"));
    }

    #[test]
    fn test_default_summarize_prompt_contains_placeholders() {
        assert!(defaults::SUMMARIZE_PROMPT.contains("{progress_file}"));
        assert!(defaults::SUMMARIZE_PROMPT.contains("{progress_content}"));
    }

    #[test]
    fn test_default_summarize_prompt_has_key_instructions() {
        // Key instructions should be present
        assert!(defaults::SUMMARIZE_PROMPT.contains("PRESERVES"));
        assert!(defaults::SUMMARIZE_PROMPT.contains("REMOVES"));
        assert!(defaults::SUMMARIZE_PROMPT.contains("200-500 lines"));
    }

    #[test]
    fn test_substitute_summarize_placeholders() {
        let template = "Path: {progress_file}\nContent: {progress_content}";
        let result =
            substitute_summarize_placeholders(template, "/path/to/progress.txt", "line 1\nline 2");

        assert_eq!(
            result,
            "Path: /path/to/progress.txt\nContent: line 1\nline 2"
        );
    }

    #[test]
    fn test_substitute_summarize_placeholders_with_default_template() {
        let result = substitute_summarize_placeholders(
            defaults::SUMMARIZE_PROMPT,
            "/project/progress.txt",
            "## Entry 1\nDid some work",
        );

        assert!(result.contains("/project/progress.txt"));
        assert!(result.contains("## Entry 1\nDid some work"));
        assert!(!result.contains("{progress_file}"));
        assert!(!result.contains("{progress_content}"));
    }

    #[test]
    fn test_substitute_summarize_preserves_unknown_placeholders() {
        let template = "{progress_file} {unknown} {another}";
        let result = substitute_summarize_placeholders(template, "/path", "content");

        assert!(result.contains("/path"));
        assert!(result.contains("{unknown}"));
        assert!(result.contains("{another}"));
    }

    #[test]
    fn test_substitute_summarize_multiple_occurrences() {
        let template = "{progress_file} repeated {progress_file}";
        let result = substitute_summarize_placeholders(template, "/path", "");

        assert_eq!(result, "/path repeated /path");
    }
}
