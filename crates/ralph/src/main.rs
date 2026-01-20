// Deny .unwrap() and .expect() in non-test code to ensure proper error handling.
// Test code may still use them for brevity.
// Any intentional uses must be documented with #[allow(...)] and comments.
#![cfg_attr(not(test), deny(clippy::unwrap_used))]
#![cfg_attr(not(test), deny(clippy::expect_used))]
// Functions should have at most 5 arguments. Use config/options structs for more.
// Threshold configured in clippy.toml (too-many-arguments-threshold = 5).
#![cfg_attr(not(test), deny(clippy::too_many_arguments))]

mod cli;
pub mod config;
pub mod diff_highlight;
mod git;
pub mod highlight;
mod init;
pub mod iteration;
pub mod iterations;
pub mod markdown;
pub mod paths;
mod prompt;
pub mod render;
pub mod replay;
pub mod replay_countdown;
pub mod replay_renderer;
mod run;
mod session;
pub mod signal;
pub mod spinner;
pub mod startup;
pub mod stream_processor;
pub mod subprocess;
pub mod summarize;

use clap::Parser;
use cli::{Cli, Commands, IterationsArgs, ReplayArgs, RunArgs, SessionsArgs};
use prompt::{prompt_on_failure, FailureAction};
use ralph_core::context::{defaults, substitute_template_placeholders, ContextPaths};
use ralph_core::session::SessionOutcome;
use run::{run, RunConfig, RunError};
use std::path::Path;
use std::process::ExitCode;
use stream_processor::VerboseToolsConfig;
use summarize::SummarizeConfig;

fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Run(args) => execute_run(args),
        Commands::Sessions(args) => execute_sessions(args),
        Commands::Iterations(args) => execute_iterations(args),
        Commands::Replay(args) => execute_replay(args),
        Commands::Themes => execute_themes(),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {}", e);
            ExitCode::FAILURE
        }
    }
}

/// Execute the run command.
fn execute_run(args: RunArgs) -> Result<(), Box<dyn std::error::Error>> {
    // Initialize signal handler for graceful shutdown on Ctrl+C/SIGTERM
    if let Err(e) = signal::init() {
        eprintln!("Warning: Failed to initialize signal handler: {}", e);
    }

    // Resolve context file paths
    let project_root = std::env::current_dir()?;
    let context_paths = ContextPaths::new(
        &project_root,
        args.design.as_deref(),
        args.prd.as_deref(),
        args.progress.as_deref(),
    );

    // Determine command template
    let command_template = args
        .command
        .clone()
        .unwrap_or_else(|| defaults::COMMAND_TEMPLATE.to_string());

    // Determine completion marker
    let completion_marker = args
        .completion_marker
        .clone()
        .unwrap_or_else(|| defaults::COMPLETION_MARKER.to_string());

    // Resolve additional prompt (file, stdin, inline, or empty)
    let additional_prompt = resolve_additional_prompt(args.additional_prompt.as_deref())?;

    // Resolve prompt template and substitute placeholders (including additional_prompt)
    let prompt = resolve_prompt(
        args.prompt.as_deref(),
        &context_paths,
        &completion_marker,
        &additional_prompt,
    )?;

    // Substitute {prompt} in command template
    let command = substitute_prompt_in_command(&command_template, &prompt);

    // Execute run loop with failure recovery prompting
    execute_run_with_prompting(args, context_paths, command, prompt, completion_marker)
}

/// Execute run loop with interactive failure recovery prompting.
///
/// This function handles the case where the LLM subprocess fails after exhausting
/// all automatic recovery attempts. If stdin is interactive, it prompts the user
/// to either continue recovery or abort. Non-interactive sessions abort automatically.
///
/// When the user chooses to continue recovery, the same session is continued
/// rather than creating a new one. The session slug is preserved across recovery
/// attempts, and iterations are aggregated within the same session.
fn execute_run_with_prompting(
    args: RunArgs,
    context_paths: ContextPaths,
    command: String,
    prompt: String,
    completion_marker: String,
) -> Result<(), Box<dyn std::error::Error>> {
    // Track the session slug across recovery attempts. Once a session is created, we reuse it.
    let mut current_session_slug: Option<String> = None;
    // Track total iterations completed across recovery attempts within the same session.
    let mut total_iterations_completed: usize = 0;

    // Build theme configuration from config file, env vars, and CLI args
    // Priority: CLI flag > environment variable > config file > default
    let theme_config = Some(
        highlight::ThemeConfig::from_config_and_env()
            .merge_cli(args.theme.as_deref(), args.no_background),
    );

    // Track custom config flags for startup display
    let custom_prd_path = args.prd.clone();
    let custom_design_path = args.design.clone();
    let custom_progress_path = args.progress.clone();
    let custom_command = args.command.is_some();
    let custom_prompt = args.prompt.is_some();
    let custom_completion_marker = args.completion_marker.is_some();
    let custom_additional_prompt = args.additional_prompt.is_some();

    // Build summarization config from CLI args
    let summarize_config = build_summarize_config(&args)?;

    // Parse verbose tools config from CLI args
    let verbose_tools_config = VerboseToolsConfig::from_arg(args.verbose_tools.as_deref());
    // Print warnings about unknown tool names
    for warning in verbose_tools_config.warnings() {
        eprintln!("Warning: {}", warning);
    }

    loop {
        // Build run config, using the established session slug if we have one
        let config = RunConfig {
            max_iterations: args.iterations,
            slug: current_session_slug.clone().or_else(|| args.slug.clone()),
            command: command.clone(),
            prompt: prompt.clone(),
            completion_marker: completion_marker.clone(),
            context_paths: context_paths.clone(),
            max_attempts: args.max_attempts,
            // Pass the starting iteration number for session continuation
            starting_iteration: total_iterations_completed,
            timeout_secs: args.timeout,
            theme_config: theme_config.clone(),
            custom_prd_path: custom_prd_path.clone(),
            custom_design_path: custom_design_path.clone(),
            custom_progress_path: custom_progress_path.clone(),
            custom_command,
            custom_prompt,
            custom_completion_marker,
            custom_additional_prompt,
            summarize_config: summarize_config.clone(),
            verbose_tools_config: verbose_tools_config.clone(),
            show_prompt: !args.no_prompt,
        };

        // Execute the run loop
        match run(config) {
            Ok(result) => {
                // Success - display final run summary
                let total_iterations = total_iterations_completed + result.iterations_completed;
                let run_summary = startup::RunSummary {
                    slug: result.slug,
                    iterations_completed: total_iterations,
                    completion_reason: result.completion_reason.map(|r| format!("{:?}", r)),
                    total_cost_usd: result.total_cost_usd,
                    total_duration_ms: result.total_duration_ms,
                    total_input_tokens: result.total_input_tokens,
                    total_output_tokens: result.total_output_tokens,
                    final_pending_stories: result.final_pending_stories,
                };
                startup::display_run_summary(&run_summary);
                return Ok(());
            }
            Err(RunError::SubprocessFailed {
                exit_code,
                attempts,
                raw_text: _,
                stderr: _,
                session_slug,
                iterations_completed,
            }) => {
                // Track the session slug for potential recovery
                if current_session_slug.is_none() {
                    current_session_slug = Some(session_slug.clone());
                }

                // Subprocess failed after exhausting all attempts - prompt user
                let summary = format!(
                    "LLM subprocess failed with exit code {} after {} attempt(s).",
                    exit_code, attempts
                );

                match prompt_on_failure(&summary) {
                    Some(FailureAction::Retry) => {
                        // Continue the same session - don't finalize, just accumulate iterations
                        total_iterations_completed += iterations_completed;
                        eprintln!("\nContinuing run (session '{}')...\n", session_slug);
                        // Continue loop with the same session
                        continue;
                    }
                    Some(FailureAction::Abort) => {
                        // User chose to abort - finalize session as aborted
                        let final_iterations = total_iterations_completed + iterations_completed;
                        if let Err(e) = session::finalize_session(
                            &session_slug,
                            final_iterations as u32,
                            SessionOutcome::Aborted,
                        ) {
                            eprintln!("Warning: Failed to finalize session: {}", e);
                        }
                        return Err("Aborted by user".into());
                    }
                    None => {
                        // Non-interactive mode or EOF - finalize as failed and abort
                        let final_iterations = total_iterations_completed + iterations_completed;
                        if let Err(e) = session::finalize_session(
                            &session_slug,
                            final_iterations as u32,
                            SessionOutcome::Failed,
                        ) {
                            eprintln!("Warning: Failed to finalize session: {}", e);
                        }
                        eprintln!("Non-interactive mode - aborting.");
                        return Err(format!(
                            "LLM subprocess failed with exit code {} after {} attempt(s)",
                            exit_code, attempts
                        )
                        .into());
                    }
                }
            }
            Err(RunError::SubprocessTimedOut {
                timeout_secs,
                attempts,
                raw_text: _,
                stderr: _,
                session_slug,
                iterations_completed,
            }) => {
                // Track the session slug for potential recovery
                if current_session_slug.is_none() {
                    current_session_slug = Some(session_slug.clone());
                }

                // Subprocess timed out after exhausting all attempts - prompt user
                let summary = format!(
                    "LLM subprocess timed out after {} seconds ({} attempt(s)).",
                    timeout_secs, attempts
                );

                match prompt_on_failure(&summary) {
                    Some(FailureAction::Retry) => {
                        // Continue the same session - don't finalize, just accumulate iterations
                        total_iterations_completed += iterations_completed;
                        eprintln!("\nContinuing run (session '{}')...\n", session_slug);
                        // Continue loop with the same session
                        continue;
                    }
                    Some(FailureAction::Abort) => {
                        // User chose to abort - finalize session as aborted
                        let final_iterations = total_iterations_completed + iterations_completed;
                        if let Err(e) = session::finalize_session(
                            &session_slug,
                            final_iterations as u32,
                            SessionOutcome::Aborted,
                        ) {
                            eprintln!("Warning: Failed to finalize session: {}", e);
                        }
                        return Err("Aborted by user".into());
                    }
                    None => {
                        // Non-interactive mode or EOF - finalize as failed and abort
                        let final_iterations = total_iterations_completed + iterations_completed;
                        if let Err(e) = session::finalize_session(
                            &session_slug,
                            final_iterations as u32,
                            SessionOutcome::Failed,
                        ) {
                            eprintln!("Warning: Failed to finalize session: {}", e);
                        }
                        eprintln!("Non-interactive mode - aborting.");
                        return Err(format!(
                            "LLM subprocess timed out after {} seconds ({} attempt(s))",
                            timeout_secs, attempts
                        )
                        .into());
                    }
                }
            }
            Err(RunError::Interrupted {
                session_slug,
                iterations_completed,
                partial_result,
                pending_before,
            }) => {
                // Run was interrupted by signal
                // If we have a partial result (interrupt during subprocess), write partial iteration log
                if let (Some(partial), Some(pending)) = (partial_result, pending_before) {
                    // Calculate iteration number
                    let iteration = total_iterations_completed + iterations_completed + 1;
                    let session_dir = session::session_dir(&session_slug);

                    // Write partial iteration log with whatever output blocks were accumulated
                    let partial_log = iteration::IterationLog {
                        sequence: iteration as u32,
                        started_at: chrono::Utc::now(),
                        completed_at: chrono::Utc::now(),
                        exit_code: partial.exit_code,
                        pending_before: pending,
                        pending_after: pending, // Same as before since iteration was interrupted
                        metadata: iteration::LogMetadata::from_extracted(
                            partial.stream_result.metadata.clone(),
                            partial.stream_result.costs.clone(),
                        )
                        .into_option(),
                        tool_calls: iteration::LogToolCall::from_interactions(
                            &partial.stream_result.tool_interactions,
                        ),
                        chunks: iteration::Chunk::from_parsed_chunks(&partial.stream_result.chunks),
                        output_blocks: partial.stream_result.output_blocks.clone(),
                    };

                    if let Err(e) = iteration::write_iteration_log(&session_dir, &partial_log) {
                        eprintln!("Warning: Failed to write partial iteration log: {}", e);
                    } else {
                        eprintln!("Partial iteration {} saved.", iteration);
                    }
                }

                // Finalize session as interrupted
                let final_iterations = total_iterations_completed + iterations_completed;
                if let Err(e) = session::finalize_session(
                    &session_slug,
                    final_iterations as u32,
                    SessionOutcome::Interrupted,
                ) {
                    eprintln!("Warning: Failed to finalize session: {}", e);
                }
                eprintln!("Interrupted. Session '{}' saved.", session_slug);
                return Err("Interrupted by signal".into());
            }
            Err(e) => {
                // Other errors - propagate immediately
                return Err(e.into());
            }
        }
    }
}

/// Execute the sessions command.
///
/// Lists all sessions from the global sessions index, optionally filtered
/// by project path or outcome status.
fn execute_sessions(args: SessionsArgs) -> Result<(), Box<dyn std::error::Error>> {
    let index = session::load_sessions_index()?;

    // Filter sessions based on arguments
    let mut sessions: Vec<_> = index
        .sessions
        .iter()
        .filter(|s| {
            // Filter by project if specified
            if let Some(ref project_filter) = args.project {
                if !s.project.display().to_string().contains(project_filter) {
                    return false;
                }
            }

            // Filter by outcome if specified
            if let Some(ref outcome_filter) = args.outcome {
                let outcome_str = s.outcome.to_string();
                if !outcome_str.eq_ignore_ascii_case(outcome_filter) {
                    return false;
                }
            }

            true
        })
        .collect();

    // Sort by date (most recent first)
    sessions.sort_by(|a, b| b.started_at.cmp(&a.started_at));

    if sessions.is_empty() {
        if args.project.is_some() || args.outcome.is_some() {
            println!("No sessions found matching the specified filters.");
        } else {
            println!("No sessions found. Run 'ralph run' to start a session.");
        }
        return Ok(());
    }

    // Print header
    println!(
        "{:<20} {:<40} {:<20} {:<6} {:<12}",
        "SLUG", "PROJECT", "DATE", "ITERS", "OUTCOME"
    );
    println!("{}", "-".repeat(100));

    // Print sessions
    for session in &sessions {
        // Truncate project path if too long
        let project_str = session.project.display().to_string();
        let project_display = if project_str.len() > 38 {
            format!("...{}", &project_str[project_str.len() - 35..])
        } else {
            project_str
        };

        // Format date
        let date_str = session.started_at.format("%Y-%m-%d %H:%M").to_string();

        // Format outcome with color hints (no actual ANSI for now, keeping it simple)
        let outcome_str = match session.outcome {
            SessionOutcome::Completed => "completed",
            SessionOutcome::InProgress => "in_progress",
            SessionOutcome::Aborted => "aborted",
            SessionOutcome::Failed => "failed",
            SessionOutcome::Interrupted => "interrupted",
        };

        println!(
            "{:<20} {:<40} {:<20} {:<6} {:<12}",
            session.slug, project_display, date_str, session.iterations, outcome_str
        );
    }

    // Print summary
    println!();
    println!("Total: {} session(s)", sessions.len());

    Ok(())
}

/// Execute the iterations command.
///
/// Lists all iterations across all sessions, optionally filtered by
/// session, project, or outcome.
fn execute_iterations(args: IterationsArgs) -> Result<(), Box<dyn std::error::Error>> {
    let filter = iterations::IterationsFilter {
        session: args.session,
        project: args.project,
        outcome: args.outcome,
    };

    iterations::list_iterations(filter)?;

    Ok(())
}

/// Execute the replay command.
///
/// Replays a session's output with syntax highlighting. Reads iteration logs
/// from the session directory and re-renders the chunks.
fn execute_replay(args: ReplayArgs) -> Result<(), Box<dyn std::error::Error>> {
    // Build theme configuration from config file, env vars, and CLI args
    let theme_config = Some(
        highlight::ThemeConfig::from_config_and_env()
            .merge_cli(args.theme.as_deref(), args.no_background),
    );

    let options = replay::ReplayOptions::new()
        .with_iteration(args.iteration)
        .with_theme(theme_config)
        .with_show_prompt(!args.no_prompt)
        .with_delay(args.delay);

    let result = replay::replay_session_with_options(&args.slug, options)?;

    // Print summary
    println!();
    println!("{}", "─".repeat(60));
    println!(
        "Replayed {} iteration(s) from session '{}'",
        result.iterations_replayed, result.slug
    );

    Ok(())
}

/// Execute the themes command.
///
/// Lists all available syntax highlighting themes.
fn execute_themes() -> Result<(), Box<dyn std::error::Error>> {
    let themes = highlight::Highlighter::available_themes();

    println!("Available syntax highlighting themes:\n");
    for theme in &themes {
        if *theme == highlight::DEFAULT_THEME {
            println!("  {} (default)", theme);
        } else {
            println!("  {}", theme);
        }
    }

    println!();
    println!("Use --theme <NAME> to select a theme for 'ralph run'.");
    println!("Or set the RALPH_THEME environment variable.");
    println!();
    println!("You can also load custom .tmTheme files by specifying a file path.");

    Ok(())
}

/// Resolve the prompt from various sources.
///
/// Loads the prompt template from one of three sources:
/// - A file path (if the argument is a path to an existing file)
/// - Stdin (if the argument is "-")
/// - An inline string (if the argument doesn't match a file)
/// - The default template (if no argument is provided)
///
/// After loading the template, placeholders are substituted with actual values:
/// - `{design_file}` - Path to the design document
/// - `{prd_file}` - Path to the PRD file
/// - `{progress_file}` - Path to the progress notes file
/// - `{completion_marker}` - The completion marker string
/// - `{additional_prompt}` - Additional instructions appended to the prompt
fn resolve_prompt(
    prompt_arg: Option<&str>,
    context_paths: &ContextPaths,
    completion_marker: &str,
    additional_prompt: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let template = match prompt_arg {
        Some("-") => {
            // Read from stdin
            use std::io::Read;
            let mut prompt = String::new();
            std::io::stdin().read_to_string(&mut prompt)?;
            prompt
        }
        Some(value) => {
            // Check if it's a file path
            let path = Path::new(value);
            if path.exists() && path.is_file() {
                std::fs::read_to_string(path)?
            } else {
                // Treat as inline string
                value.to_string()
            }
        }
        None => {
            // Use built-in default prompt template
            defaults::PROMPT_TEMPLATE.to_string()
        }
    };

    // Substitute placeholders in the template
    Ok(substitute_template_placeholders(
        &template,
        context_paths,
        completion_marker,
        additional_prompt,
    ))
}

/// Resolve additional prompt from various sources.
///
/// Loads additional prompt instructions from:
/// - A file path (if the argument is a path to an existing file)
/// - Stdin (if the argument is "-")
/// - An inline string (if the argument doesn't match a file)
/// - Empty string (if no argument is provided)
fn resolve_additional_prompt(
    additional_arg: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    match additional_arg {
        Some("-") => {
            // Read from stdin
            use std::io::Read;
            let mut content = String::new();
            std::io::stdin().read_to_string(&mut content)?;
            Ok(content)
        }
        Some(value) => {
            // Check if it's a file path
            let path = Path::new(value);
            if path.exists() && path.is_file() {
                Ok(std::fs::read_to_string(path)?)
            } else {
                // Treat as inline string
                Ok(value.to_string())
            }
        }
        None => {
            // No additional prompt - return empty string
            Ok(String::new())
        }
    }
}

/// Substitute {prompt} placeholder in command template.
fn substitute_prompt_in_command(template: &str, prompt: &str) -> String {
    // Shell-escape the prompt for safe inclusion
    // For now, just wrap in single quotes and escape internal single quotes
    let escaped = prompt.replace('\'', "'\"'\"'");
    let quoted_prompt = format!("'{}'", escaped);
    template.replace("{prompt}", &quoted_prompt)
}

/// Build summarization config from CLI arguments.
///
/// Resolves the summarize prompt from file, stdin, inline string, or default.
fn build_summarize_config(args: &RunArgs) -> Result<SummarizeConfig, Box<dyn std::error::Error>> {
    // If --no-summarize is set, return disabled config
    if args.no_summarize {
        return Ok(SummarizeConfig {
            disabled: true,
            ..Default::default()
        });
    }

    // Resolve summarize prompt (same pattern as resolve_additional_prompt)
    let prompt = match args.summarize_prompt.as_deref() {
        Some("-") => {
            // Read from stdin
            use std::io::Read;
            let mut content = String::new();
            std::io::stdin().read_to_string(&mut content)?;
            content
        }
        Some(value) => {
            // Check if it's a file path
            let path = Path::new(value);
            if path.exists() && path.is_file() {
                std::fs::read_to_string(path)?
            } else {
                // Treat as inline string
                value.to_string()
            }
        }
        None => {
            // Use default prompt
            defaults::SUMMARIZE_PROMPT.to_string()
        }
    };

    // Get command template
    let command = args
        .summarize_command
        .clone()
        .unwrap_or_else(|| defaults::SUMMARIZE_COMMAND.to_string());

    Ok(SummarizeConfig {
        max_lines: args.progress_max_lines,
        command,
        prompt,
        disabled: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_substitute_prompt_simple() {
        let result = substitute_prompt_in_command("echo {prompt}", "hello");
        assert_eq!(result, "echo 'hello'");
    }

    #[test]
    fn test_substitute_prompt_with_quotes() {
        let result = substitute_prompt_in_command("echo {prompt}", "it's a test");
        assert_eq!(result, "echo 'it'\"'\"'s a test'");
    }

    #[test]
    fn test_substitute_prompt_claude_command() {
        let result = substitute_prompt_in_command(
            "claude --permission-mode acceptEdits --output-format stream-json -p {prompt}",
            "test prompt",
        );
        assert_eq!(
            result,
            "claude --permission-mode acceptEdits --output-format stream-json -p 'test prompt'"
        );
    }

    #[test]
    fn test_substitute_prompt_custom_command_structure() {
        // Custom commands can use any structure
        let result = substitute_prompt_in_command("my-llm --input {prompt} --verbose", "hello");
        assert_eq!(result, "my-llm --input 'hello' --verbose");
    }

    #[test]
    fn test_substitute_prompt_custom_command_with_env_vars() {
        // Custom command with environment variables and complex structure
        let result =
            substitute_prompt_in_command("OPENAI_KEY=$KEY openai-cli chat {prompt}", "query");
        assert_eq!(result, "OPENAI_KEY=$KEY openai-cli chat 'query'");
    }

    #[test]
    fn test_substitute_prompt_empty_prompt() {
        let result = substitute_prompt_in_command("echo {prompt}", "");
        assert_eq!(result, "echo ''");
    }

    #[test]
    fn test_substitute_prompt_multiline() {
        let result = substitute_prompt_in_command("echo {prompt}", "line1\nline2");
        assert_eq!(result, "echo 'line1\nline2'");
    }

    #[test]
    fn test_substitute_prompt_special_shell_chars() {
        // Shell special characters should be safely escaped within single quotes
        let result = substitute_prompt_in_command("echo {prompt}", "test $VAR `cmd` $(cmd)");
        assert_eq!(result, "echo 'test $VAR `cmd` $(cmd)'");
    }

    #[test]
    fn test_substitute_prompt_no_placeholder() {
        // Command without {prompt} placeholder returns unchanged
        let result = substitute_prompt_in_command("echo hello", "ignored");
        assert_eq!(result, "echo hello");
    }

    #[test]
    fn test_default_command_template_substitution() {
        // Verify the default command template works correctly
        let result = substitute_prompt_in_command(defaults::COMMAND_TEMPLATE, "my prompt");
        assert!(result.starts_with("claude "));
        assert!(result.contains("--output-format stream-json"));
        assert!(result.contains("'my prompt'"));
    }
}
