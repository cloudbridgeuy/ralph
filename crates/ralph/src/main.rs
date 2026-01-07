mod cli;
pub mod diff_highlight;
mod git;
pub mod highlight;
mod init;
pub mod iteration;
mod prompt;
pub mod replay;
mod run;
mod session;
pub mod stream_processor;
pub mod subprocess;

use clap::Parser;
use cli::{Cli, Commands, ReplayArgs, RunArgs, SessionsArgs};
use prompt::{prompt_on_failure, FailureAction};
use ralph_core::context::{defaults, substitute_template_placeholders, ContextPaths};
use ralph_core::session::SessionOutcome;
use run::{run, RunConfig, RunError};
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Run(args) => execute_run(args),
        Commands::Sessions(args) => execute_sessions(args),
        Commands::Replay(args) => execute_replay(args),
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

    // Resolve prompt template and substitute placeholders
    let prompt = resolve_prompt(args.prompt.as_deref(), &context_paths, &completion_marker)?;

    // Substitute {prompt} in command template
    let command = substitute_prompt_in_command(&command_template, &prompt);

    // Execute run loop with retry prompting on failure
    execute_run_with_prompting(args, context_paths, command, completion_marker)
}

/// Execute run loop with interactive retry prompting on unrecoverable failures.
///
/// This function handles the case where the LLM subprocess fails after exhausting
/// all automatic retries. If stdin is interactive, it prompts the user to either
/// retry the entire run or abort. Non-interactive sessions abort automatically.
///
/// When the user chooses to retry, the same session is continued rather than
/// creating a new one. The session slug is preserved across retries, and
/// iterations are aggregated within the same session.
fn execute_run_with_prompting(
    args: RunArgs,
    context_paths: ContextPaths,
    command: String,
    completion_marker: String,
) -> Result<(), Box<dyn std::error::Error>> {
    // Track the session slug across retries. Once a session is created, we reuse it.
    let mut current_session_slug: Option<String> = None;
    // Track total iterations completed across retries within the same session.
    let mut total_iterations_completed: usize = 0;

    loop {
        // Build run config, using the established session slug if we have one
        let config = RunConfig {
            max_iterations: args.iterations,
            slug: current_session_slug.clone().or_else(|| args.slug.clone()),
            command: command.clone(),
            completion_marker: completion_marker.clone(),
            context_paths: context_paths.clone(),
            retry_count: args.retry,
            // Pass the starting iteration number for session continuation
            starting_iteration: total_iterations_completed,
            timeout_secs: args.timeout,
        };

        // Execute the run loop
        match run(config) {
            Ok(result) => {
                // Success - print summary and return
                println!("Session: {}", result.slug);
                println!(
                    "Iterations completed: {}",
                    total_iterations_completed + result.iterations_completed
                );
                if let Some(reason) = result.completion_reason {
                    println!("Completion reason: {:?}", reason);
                }
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
                // Track the session slug for potential retry
                if current_session_slug.is_none() {
                    current_session_slug = Some(session_slug.clone());
                }

                // Subprocess failed after exhausting retries - prompt user
                let summary = format!(
                    "LLM subprocess failed with exit code {} after {} attempt(s).",
                    exit_code, attempts
                );

                match prompt_on_failure(&summary) {
                    Some(FailureAction::Retry) => {
                        // Continue the same session on retry - don't finalize, just accumulate iterations
                        total_iterations_completed += iterations_completed;
                        eprintln!(
                            "\nRetrying run (continuing session '{}')...\n",
                            session_slug
                        );
                        // Continue loop to retry with the same session
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
                // Track the session slug for potential retry
                if current_session_slug.is_none() {
                    current_session_slug = Some(session_slug.clone());
                }

                // Subprocess timed out after exhausting retries - prompt user
                let summary = format!(
                    "LLM subprocess timed out after {} seconds ({} attempt(s)).",
                    timeout_secs, attempts
                );

                match prompt_on_failure(&summary) {
                    Some(FailureAction::Retry) => {
                        // Continue the same session on retry - don't finalize, just accumulate iterations
                        total_iterations_completed += iterations_completed;
                        eprintln!(
                            "\nRetrying run (continuing session '{}')...\n",
                            session_slug
                        );
                        // Continue loop to retry with the same session
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

/// Execute the replay command.
///
/// Replays a session's output with syntax highlighting. Reads iteration logs
/// from the session directory and re-renders the chunks.
fn execute_replay(args: ReplayArgs) -> Result<(), Box<dyn std::error::Error>> {
    let result = replay::replay_session(&args.slug, args.iteration)?;

    // Print summary
    println!();
    println!("{}", "─".repeat(60));
    println!(
        "Replayed {} iteration(s) from session '{}'",
        result.iterations_replayed, result.slug
    );

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
fn resolve_prompt(
    prompt_arg: Option<&str>,
    context_paths: &ContextPaths,
    completion_marker: &str,
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
    ))
}

/// Substitute {prompt} placeholder in command template.
fn substitute_prompt_in_command(template: &str, prompt: &str) -> String {
    // Shell-escape the prompt for safe inclusion
    // For now, just wrap in single quotes and escape internal single quotes
    let escaped = prompt.replace('\'', "'\"'\"'");
    let quoted_prompt = format!("'{}'", escaped);
    template.replace("{prompt}", &quoted_prompt)
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
