//! Context file path resolution and template placeholders.
//!
//! This module provides pure functions for substituting placeholders in
//! prompt templates. Following the Functional Core pattern, all functions
//! operate on data provided as arguments - no file I/O.

use std::path::Path;

/// Default paths and templates for context files and commands.
pub mod defaults {
    /// Default path for the PRD file.
    pub const PRD_FILE: &str = ".local/plans/prd.toml";

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
    /// - `{prd_file}` - Path to the PRD file
    /// - `{completion_marker}` - The marker string to output when all stories are complete
    /// - `{additional_prompt}` - Additional instructions appended to the prompt (optional)
    ///
    /// # Notes
    ///
    /// The template uses `@` file references which are understood by Claude CLI
    /// to read and include the file contents in the context.
    ///
    /// # Workflow
    ///
    /// The prompt enforces a two-phase workflow:
    /// 1. **Work Phase**: Implement the feature with lint checks
    /// 2. **Commit Phase**: Update documentation per Progressive Disclosure, then commit
    ///
    /// Context retrieval uses:
    /// - `git log --oneline -10` for recent commit history
    /// - CLAUDE.md and docs/context/*.md for architectural context (Progressive Disclosure)
    pub const PROMPT_TEMPLATE: &str = r#"@{prd_file}

## Context Retrieval

Before starting work:
1. Run `git log --oneline -10` to understand recent changes and commit style
2. Read CLAUDE.md for project conventions and architectural overview
3. Consult linked docs/context/*.md files for detailed context on specific areas

## Phase 1: Work

1. Find the highest-priority feature to work on and implement it.
   This should be the one YOU decide has the highest priority - not necessarily the first in the list.

2. Run 'cargo xtask lint' and ensure it passes.
   You cannot mark a user story as complete if this command fails.

3. Update the PRD with the work that was done by setting passes = true for completed stories.

4. If you find the PRD is missing information to complete your task, append it using the appropriate format.

ONLY WORK ON A SINGLE FEATURE.

## Phase 2: Commit

After the work is complete and lint passes:

1. Update documentation following the Progressive Disclosure pattern:
   - If your changes affect project conventions, update CLAUDE.md
   - If your changes add detailed context for a specific area, create or update docs/context/*.md
   - Link any new docs/context/*.md files from CLAUDE.md

2. Make a semantic commit without Claude attribution:
   - Use conventional commit format: feat|fix|refactor|docs|test|chore(scope): description
   - The commit message should explain the "why" not just the "what"

{additional_prompt}

If all stories in the PRD are complete, output {completion_marker}."#;
}

/// Resolve the PRD path, using the override if provided or the default path.
///
/// # Arguments
///
/// * `project_root` - Base directory for resolving relative paths
/// * `prd_override` - Optional override for PRD file path
///
/// # Returns
///
/// The resolved PRD path.
pub fn resolve_prd_path(project_root: &Path, prd_override: Option<&Path>) -> std::path::PathBuf {
    prd_override
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| project_root.join(defaults::PRD_FILE))
}

/// Substitute placeholders in a prompt template.
///
/// This is a pure function that replaces known placeholders with their values.
/// Unknown placeholders are left unchanged.
///
/// # Placeholders
///
/// - `{prd_file}` - Replaced with the PRD file path
/// - `{completion_marker}` - Replaced with the completion marker string
/// - `{additional_prompt}` - Replaced with additional prompt instructions
///
/// # Arguments
///
/// * `template` - The prompt template containing placeholders
/// * `prd_path` - Path to the PRD file
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
/// use ralph_core::context::substitute_template_placeholders;
/// use std::path::Path;
///
/// let template = "Read @{prd_file}";
/// let result = substitute_template_placeholders(
///     template,
///     Path::new("/project/.local/plans/prd.toml"),
///     "<promise>COMPLETE</promise>",
///     ""
/// );
///
/// assert!(result.contains("/project/.local/plans/prd.toml"));
/// ```
pub fn substitute_template_placeholders(
    template: &str,
    prd_path: &Path,
    completion_marker: &str,
    additional_prompt: &str,
) -> String {
    template
        .replace("{prd_file}", &prd_path.display().to_string())
        .replace("{completion_marker}", completion_marker)
        .replace("{additional_prompt}", additional_prompt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // Tests for resolve_prd_path

    #[test]
    fn test_resolve_prd_path_default() {
        let root = Path::new("/project");
        let path = resolve_prd_path(root, None);
        assert_eq!(path, PathBuf::from("/project/.local/plans/prd.toml"));
    }

    #[test]
    fn test_resolve_prd_path_with_override() {
        let root = Path::new("/project");
        let path = resolve_prd_path(root, Some(Path::new("/custom/prd.toml")));
        assert_eq!(path, PathBuf::from("/custom/prd.toml"));
    }

    // Tests for defaults

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
    fn test_default_prompt_template_contains_prd_placeholder() {
        assert!(defaults::PROMPT_TEMPLATE.contains("{prd_file}"));
    }

    #[test]
    fn test_default_prompt_template_does_not_contain_design_placeholder() {
        // Design file is no longer part of the simplified context model
        assert!(!defaults::PROMPT_TEMPLATE.contains("{design_file}"));
    }

    #[test]
    fn test_default_prompt_template_does_not_contain_progress_placeholder() {
        // Progress file is no longer part of the simplified context model
        assert!(!defaults::PROMPT_TEMPLATE.contains("{progress_file}"));
    }

    #[test]
    fn test_default_prompt_template_contains_completion_marker_placeholder() {
        assert!(defaults::PROMPT_TEMPLATE.contains("{completion_marker}"));
    }

    #[test]
    fn test_default_prompt_template_uses_at_file_references() {
        // Claude CLI uses @ for file references - only PRD is referenced now
        assert!(defaults::PROMPT_TEMPLATE.contains("@{prd_file}"));
        assert!(!defaults::PROMPT_TEMPLATE.contains("@{design_file}"));
        assert!(!defaults::PROMPT_TEMPLATE.contains("@{progress_file}"));
    }

    #[test]
    fn test_default_prompt_template_includes_key_instructions() {
        // Key workflow instructions should be present
        assert!(defaults::PROMPT_TEMPLATE.contains("highest-priority feature"));
        assert!(defaults::PROMPT_TEMPLATE.contains("passes = true"));
        assert!(defaults::PROMPT_TEMPLATE.contains("ONLY WORK ON A SINGLE FEATURE"));
    }

    #[test]
    fn test_default_prompt_template_includes_git_log_instruction() {
        // Git history should be used for context
        assert!(defaults::PROMPT_TEMPLATE.contains("git log"));
    }

    #[test]
    fn test_default_prompt_template_includes_progressive_disclosure() {
        // Progressive Disclosure pattern should be referenced
        assert!(defaults::PROMPT_TEMPLATE.contains("CLAUDE.md"));
        assert!(defaults::PROMPT_TEMPLATE.contains("docs/context/"));
    }

    #[test]
    fn test_default_prompt_template_includes_semantic_commit() {
        // Semantic commit format should be mentioned
        assert!(defaults::PROMPT_TEMPLATE.contains("semantic commit"));
    }

    #[test]
    fn test_default_prompt_template_has_phase_separation() {
        // Two-phase workflow should be clear
        assert!(defaults::PROMPT_TEMPLATE.contains("Phase 1: Work"));
        assert!(defaults::PROMPT_TEMPLATE.contains("Phase 2: Commit"));
    }

    // Tests for substitute_template_placeholders function

    #[test]
    fn test_substitute_all_placeholders() {
        let prd_path = Path::new("/project/prd.toml");
        let template = "{prd_file} {completion_marker} {additional_prompt}";
        let result = substitute_template_placeholders(template, prd_path, "DONE", "extra");

        assert_eq!(result, "/project/prd.toml DONE extra");
    }

    #[test]
    fn test_substitute_preserves_unknown_placeholders() {
        let prd_path = Path::new("/prd.toml");
        let template = "{prd_file} {unknown_placeholder} {another}";
        let result = substitute_template_placeholders(template, prd_path, "DONE", "");

        assert!(result.contains("/prd.toml"));
        assert!(result.contains("{unknown_placeholder}"));
        assert!(result.contains("{another}"));
    }

    #[test]
    fn test_substitute_with_default_prompt_template() {
        let prd_path = Path::new("/my/prd.toml");
        let result =
            substitute_template_placeholders(defaults::PROMPT_TEMPLATE, prd_path, "COMPLETE", "");

        // Only PRD is referenced in the new template
        assert!(result.contains("@/my/prd.toml"));
        assert!(result.contains("output COMPLETE"));
        // No remaining placeholders for known values
        assert!(!result.contains("{prd_file}"));
        assert!(!result.contains("{completion_marker}"));
    }

    #[test]
    fn test_substitute_empty_template() {
        let prd_path = Path::new("/prd.toml");
        let result = substitute_template_placeholders("", prd_path, "DONE", "");
        assert_eq!(result, "");
    }

    #[test]
    fn test_substitute_no_placeholders() {
        let prd_path = Path::new("/prd.toml");
        let template = "No placeholders here";
        let result = substitute_template_placeholders(template, prd_path, "DONE", "");
        assert_eq!(result, "No placeholders here");
    }

    #[test]
    fn test_substitute_multiple_prd_occurrences() {
        let prd_path = Path::new("/prd.toml");
        let template = "{prd_file} and also {prd_file}";
        let result = substitute_template_placeholders(template, prd_path, "DONE", "");
        assert_eq!(result, "/prd.toml and also /prd.toml");
    }

    #[test]
    fn test_substitute_with_special_characters_in_path() {
        let prd_path = Path::new("/special!@#$/prd.toml");
        let template = "{prd_file}";
        let result = substitute_template_placeholders(template, prd_path, "DONE", "");

        assert!(result.contains("/special!@#$/prd.toml"));
    }

    #[test]
    fn test_substitute_additional_prompt() {
        let prd_path = Path::new("/prd.toml");
        let template = "Main instructions\n\n{additional_prompt}";
        let result = substitute_template_placeholders(
            template,
            prd_path,
            "DONE",
            "Custom extra instructions",
        );

        assert!(result.contains("Main instructions"));
        assert!(result.contains("Custom extra instructions"));
        assert!(!result.contains("{additional_prompt}"));
    }

    #[test]
    fn test_substitute_empty_additional_prompt() {
        let prd_path = Path::new("/prd.toml");
        let template = "Instructions{additional_prompt}";
        let result = substitute_template_placeholders(template, prd_path, "DONE", "");

        assert_eq!(result, "Instructions");
    }
}
