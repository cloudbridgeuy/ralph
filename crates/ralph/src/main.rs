// Deny .unwrap() and .expect() in non-test code to ensure proper error handling.
// Test code may still use them for brevity.
// Any intentional uses must be documented with #[allow(...)] and comments.
#![cfg_attr(not(test), deny(clippy::unwrap_used))]
#![cfg_attr(not(test), deny(clippy::expect_used))]
// Functions should have at most 5 arguments. Use config/options structs for more.
// Threshold configured in clippy.toml (too-many-arguments-threshold = 5).
#![cfg_attr(not(test), deny(clippy::too_many_arguments))]

mod ask;
mod cli;
pub mod config;
pub mod diff_highlight;
pub mod formatting;
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
pub mod sessions_display;
pub mod signal;
pub mod spinner;
pub mod startup;
pub mod stream_processor;
pub mod subprocess;
pub mod summarize;

use clap::Parser;
use cli::{AskArgs, Cli, Commands, IterationsArgs, ReplayArgs, RunArgs, SessionsArgs};
use iteration::{extract_conversation_messages, load_session_iterations};
use prompt::{prompt_on_failure, FailureAction};
use ralph_core::context::{defaults, substitute_template_placeholders, ContextPaths};
use ralph_core::session::SessionOutcome;
use run::{run, RunConfig, RunError};
use std::path::Path;
use std::process::ExitCode;
use stream_processor::VerboseToolsConfig;
use summarize::SummarizeConfig;

/// Context for handling subprocess failure recovery.
struct FailureRecoveryContext {
    /// Summary message to display to user.
    summary: String,
    /// Session slug for this run.
    session_slug: String,
    /// Iterations completed before failure.
    iterations_completed: usize,
    /// Total iterations from previous recovery attempts.
    total_iterations_completed: usize,
}

/// Result of failure recovery handling.
enum FailureRecoveryResult {
    /// User chose to retry - continue the loop.
    Retry { updated_total_iterations: usize },
    /// Recovery was aborted (user chose or non-interactive).
    Aborted(Box<dyn std::error::Error>),
}

/// Handle subprocess failure with user prompting and session finalization.
///
/// This is a pure-ish function that handles the common failure recovery pattern:
/// 1. Update current_session_slug tracking
/// 2. Prompt user (if interactive)
/// 3. Handle Retry/Abort/None responses
/// 4. Finalize session appropriately
///
/// Returns `FailureRecoveryResult::Retry` if user wants to continue,
/// or `FailureRecoveryResult::Aborted` with the error to return.
fn handle_failure_recovery(
    ctx: &FailureRecoveryContext,
    current_session_slug: &mut Option<String>,
) -> FailureRecoveryResult {
    // Track the session slug for potential recovery
    if current_session_slug.is_none() {
        *current_session_slug = Some(ctx.session_slug.clone());
    }

    match prompt_on_failure(&ctx.summary) {
        Some(FailureAction::Retry) => {
            // Continue the same session - don't finalize, just accumulate iterations
            let updated = ctx.total_iterations_completed + ctx.iterations_completed;
            eprintln!("\nContinuing run (session '{}')...\n", ctx.session_slug);
            FailureRecoveryResult::Retry {
                updated_total_iterations: updated,
            }
        }
        Some(FailureAction::Abort) => {
            // User chose to abort - finalize session as aborted
            let final_iterations = ctx.total_iterations_completed + ctx.iterations_completed;
            if let Err(e) = session::finalize_session(
                &ctx.session_slug,
                final_iterations as u32,
                SessionOutcome::Aborted,
            ) {
                eprintln!("Warning: Failed to finalize session: {}", e);
            }
            FailureRecoveryResult::Aborted("Aborted by user".into())
        }
        None => {
            // Non-interactive mode or EOF - finalize as failed and abort
            let final_iterations = ctx.total_iterations_completed + ctx.iterations_completed;
            if let Err(e) = session::finalize_session(
                &ctx.session_slug,
                final_iterations as u32,
                SessionOutcome::Failed,
            ) {
                eprintln!("Warning: Failed to finalize session: {}", e);
            }
            eprintln!("Non-interactive mode - aborting.");
            FailureRecoveryResult::Aborted(ctx.summary.clone().into())
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Run(args) => execute_run(args),
        Commands::Sessions(args) => execute_sessions(args),
        Commands::Iterations(args) => execute_iterations(args),
        Commands::Replay(args) => execute_replay(args),
        Commands::Themes => execute_themes(),
        Commands::Ask(args) => execute_ask(args),
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
                let ctx = FailureRecoveryContext {
                    summary: format!(
                        "LLM subprocess failed with exit code {} after {} attempt(s).",
                        exit_code, attempts
                    ),
                    session_slug,
                    iterations_completed,
                    total_iterations_completed,
                };

                match handle_failure_recovery(&ctx, &mut current_session_slug) {
                    FailureRecoveryResult::Retry {
                        updated_total_iterations,
                    } => {
                        total_iterations_completed = updated_total_iterations;
                        continue;
                    }
                    FailureRecoveryResult::Aborted(err) => return Err(err),
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
                let ctx = FailureRecoveryContext {
                    summary: format!(
                        "LLM subprocess timed out after {} seconds ({} attempt(s)).",
                        timeout_secs, attempts
                    ),
                    session_slug,
                    iterations_completed,
                    total_iterations_completed,
                };

                match handle_failure_recovery(&ctx, &mut current_session_slug) {
                    FailureRecoveryResult::Retry {
                        updated_total_iterations,
                    } => {
                        total_iterations_completed = updated_total_iterations;
                        continue;
                    }
                    FailureRecoveryResult::Aborted(err) => return Err(err),
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
                        prompt: None,           // Run command doesn't track prompt per iteration
                        response: iteration::extract_response_text(
                            &partial.stream_result.output_blocks,
                        ),
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
/// Lists all sessions from the global sessions index with detailed metadata
/// (cost, duration, tokens), optionally filtered by project path or outcome.
fn execute_sessions(args: SessionsArgs) -> Result<(), Box<dyn std::error::Error>> {
    let filter = sessions_display::SessionsFilter {
        project: args.project,
        outcome: args.outcome,
    };

    sessions_display::list_sessions(filter)?;

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

/// Execute the ask command.
///
/// Sends a single-shot prompt to the LLM and displays the response.
/// Creates a session for persistence, allowing replay with `ralph replay`.
/// When --continue is used, continues an existing session instead.
/// When --clone is used, creates a new session with history from an existing one.
/// When --history is used, displays the conversation history for the session.
fn execute_ask(args: AskArgs) -> Result<(), Box<dyn std::error::Error>> {
    // Initialize signal handler for graceful shutdown on Ctrl+C/SIGTERM
    if let Err(e) = signal::init() {
        eprintln!("Warning: Failed to initialize signal handler: {}", e);
    }

    // Get current working directory as project path
    let project_path = std::env::current_dir()?;

    // Handle --history flag: requires --session or --continue
    if args.history {
        return execute_ask_with_history(&args, &project_path);
    }

    // Validate --clone requires --session or --continue to specify source
    if args.clone_session && args.session.is_none() && !args.continue_session {
        return Err(
            "--clone requires --session or --continue to specify which session to clone from"
                .into(),
        );
    }

    // Determine clone info if --clone flag is set
    let clone_info = if args.clone_session {
        Some(resolve_clone_source(&args.session, &project_path)?)
    } else {
        None
    };

    // Determine continuation info if --continue flag is set (and not cloning)
    // Clone mode always creates a new session, so it's mutually exclusive with continuation
    let continuation = if args.continue_session && !args.clone_session {
        Some(resolve_continuation(&args.session, &project_path)?)
    } else {
        None
    };

    // Build theme configuration from config file, env vars, and CLI args
    // Priority: CLI flag > environment variable > config file > default
    let theme_config = highlight::ThemeConfig::from_config_and_env()
        .merge_cli(args.theme.as_deref(), args.no_background);

    // Parse verbose tools config from CLI args
    let verbose_tools = VerboseToolsConfig::from_arg(args.verbose_tools.as_deref());
    // Print warnings about unknown tool names
    for warning in verbose_tools.warnings() {
        eprintln!("Warning: {}", warning);
    }

    // Resolve prompt from argument or stdin
    let prompt = resolve_ask_prompt(args.prompt.as_deref())?;

    // Display prompt if enabled
    if !args.no_prompt && !prompt.is_empty() {
        let prompt_display = startup::PromptDisplay::from_prompt(&prompt);
        startup::display_prompt(&prompt_display);
    }

    // Build ask config - validation happens in ask::ask()
    // When cloning, we don't pass a user slug since the new session gets auto-generated
    let config = ask::AskConfig {
        prompt,
        timeout_secs: args.timeout,
        theme_config,
        verbose_tools,
        project_path,
        slug: if args.continue_session || args.clone_session {
            None // Don't pass slug when continuing or cloning (handled separately)
        } else {
            args.session
        },
        continuation,
        clone: clone_info,
        permission_mode: ask::resolve_permission_mode(args.permission_mode.as_deref()),
    };

    // Execute the ask command
    let result = ask::ask(config)?;

    // Finalize session based on exit code
    let outcome = match result.exit_code {
        0 => SessionOutcome::Completed,
        _ => SessionOutcome::Failed,
    };
    if let Err(e) = session::finalize_session(&result.slug, result.iteration_count, outcome) {
        eprintln!("Warning: Failed to finalize session: {}", e);
    }

    // Display summary (always, even on failure)
    let summary = startup::AskSummary {
        slug: result.slug.clone(),
        success: result.exit_code == 0,
        cost_usd: result.cost_usd,
        duration_ms: result.duration_ms,
        input_tokens: result.input_tokens,
        output_tokens: result.output_tokens,
    };
    startup::display_ask_summary(&summary);

    // Return error for non-zero exit
    if result.exit_code != 0 {
        return Err(format!("LLM subprocess exited with code {}", result.exit_code).into());
    }

    Ok(())
}

/// Find a session entry by name or get the most recent for the project.
///
/// This is a shared helper for both continuation and clone operations.
///
/// # Logic
/// - If `session_name` is Some: find that specific session
/// - If `session_name` is None: find the most recent session for the project
///
/// # Errors
/// - Session not found (by name or no sessions for project)
fn find_session_entry(
    session_name: &Option<String>,
    project_path: &Path,
) -> Result<ralph_core::session::SessionEntry, Box<dyn std::error::Error>> {
    if let Some(name) = session_name {
        Ok(session::find_session_by_slug(name)?)
    } else {
        session::find_most_recent_session(project_path)?.ok_or_else(|| {
            format!(
                "No sessions found for project '{}'. Create a session first with 'ralph ask'.",
                project_path.display()
            )
            .into()
        })
    }
}

/// Resolve session continuation info based on --continue flag and optional --session.
///
/// # Errors
/// - Session not found (by name or no sessions for project)
/// - Failed to count existing iterations
fn resolve_continuation(
    session_name: &Option<String>,
    project_path: &Path,
) -> Result<ask::ContinuationInfo, Box<dyn std::error::Error>> {
    use crate::iteration::count_iterations;

    let entry = find_session_entry(session_name, project_path)?;
    let session_dir = session::session_dir(&entry.slug);
    let existing_count = count_iterations(&session_dir)?;

    Ok(ask::ContinuationInfo {
        slug: entry.slug,
        next_sequence: existing_count + 1,
        session_dir,
    })
}

/// Resolve the source session for cloning.
///
/// Unlike continuation, cloning creates a NEW session with history from the source.
///
/// # Errors
/// - Session not found (by name or no sessions for project)
fn resolve_clone_source(
    session_name: &Option<String>,
    project_path: &Path,
) -> Result<ask::CloneInfo, Box<dyn std::error::Error>> {
    let entry = find_session_entry(session_name, project_path)?;
    let source_session_dir = session::session_dir(&entry.slug);

    Ok(ask::CloneInfo {
        source_slug: entry.slug,
        source_session_dir,
    })
}

/// Execute ask command with --history flag.
///
/// Displays the conversation history for a session. If a prompt is also provided
/// with --continue, displays history then proceeds with the new prompt.
fn execute_ask_with_history(
    args: &AskArgs,
    project_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    // --history requires --session or --continue to specify which session
    if args.session.is_none() && !args.continue_session {
        return Err(
            "--history requires --session or --continue to specify which session to display".into(),
        );
    }

    // Resolve the session to display history for
    let entry = if let Some(ref name) = args.session {
        session::find_session_by_slug(name)?
    } else {
        // --continue without --session: find most recent session
        session::find_most_recent_session(project_path)?.ok_or_else(|| {
            format!(
                "No sessions found for project '{}'. Create a session first with 'ralph ask'.",
                project_path.display()
            )
        })?
    };

    // Load iteration logs and extract conversation history
    let session_dir = session::session_dir(&entry.slug);
    let logs = load_session_iterations(&session_dir)?;
    let messages = extract_conversation_messages(&logs);

    // Build and display conversation history
    let history = startup::ConversationHistory::from_messages(entry.slug.clone(), messages);
    startup::display_conversation_history(&history);

    // If a prompt was also provided with --continue, proceed with asking
    if args.continue_session && args.prompt.is_some() {
        let mut modified_args = args.clone();
        modified_args.history = false;
        return execute_ask_core(modified_args);
    }

    Ok(())
}

/// Core ask execution logic (extracted for reuse with --history flag).
fn execute_ask_core(args: AskArgs) -> Result<(), Box<dyn std::error::Error>> {
    // Get current working directory as project path
    let project_path = std::env::current_dir()?;

    // Determine clone info if --clone flag is set
    let clone_info = if args.clone_session {
        Some(resolve_clone_source(&args.session, &project_path)?)
    } else {
        None
    };

    // Determine continuation info if --continue flag is set (and not cloning)
    let continuation = if args.continue_session && !args.clone_session {
        Some(resolve_continuation(&args.session, &project_path)?)
    } else {
        None
    };

    // Build theme configuration from config file, env vars, and CLI args
    let theme_config = highlight::ThemeConfig::from_config_and_env()
        .merge_cli(args.theme.as_deref(), args.no_background);

    // Parse verbose tools config from CLI args
    let verbose_tools = VerboseToolsConfig::from_arg(args.verbose_tools.as_deref());
    for warning in verbose_tools.warnings() {
        eprintln!("Warning: {}", warning);
    }

    // Resolve prompt from argument or stdin
    let prompt = resolve_ask_prompt(args.prompt.as_deref())?;

    // Display prompt if enabled
    if !args.no_prompt && !prompt.is_empty() {
        let prompt_display = startup::PromptDisplay::from_prompt(&prompt);
        startup::display_prompt(&prompt_display);
    }

    // Build ask config
    let config = ask::AskConfig {
        prompt,
        timeout_secs: args.timeout,
        theme_config,
        verbose_tools,
        project_path,
        slug: if args.continue_session || args.clone_session {
            None
        } else {
            args.session
        },
        continuation,
        clone: clone_info,
        permission_mode: ask::resolve_permission_mode(args.permission_mode.as_deref()),
    };

    // Execute the ask command
    let result = ask::ask(config)?;

    // Finalize session based on exit code
    let outcome = match result.exit_code {
        0 => SessionOutcome::Completed,
        _ => SessionOutcome::Failed,
    };
    if let Err(e) = session::finalize_session(&result.slug, result.iteration_count, outcome) {
        eprintln!("Warning: Failed to finalize session: {}", e);
    }

    // Display summary
    let summary = startup::AskSummary {
        slug: result.slug.clone(),
        success: result.exit_code == 0,
        cost_usd: result.cost_usd,
        duration_ms: result.duration_ms,
        input_tokens: result.input_tokens,
        output_tokens: result.output_tokens,
    };
    startup::display_ask_summary(&summary);

    // Return error for non-zero exit
    if result.exit_code != 0 {
        return Err(format!("LLM subprocess exited with code {}", result.exit_code).into());
    }

    Ok(())
}

/// Input source for prompt resolution.
#[derive(Debug, Clone, PartialEq)]
enum PromptSource<'a> {
    /// Read from stdin (when arg is "-")
    Stdin,
    /// Read from file at path
    File(&'a Path),
    /// Use inline string directly
    Inline(&'a str),
    /// No input provided
    None,
}

/// Classify the input argument into a source type.
///
/// This is a pure function that determines how to interpret the argument:
/// - "-" means stdin
/// - An existing file path means read from file
/// - Any other string is treated as inline content
/// - None means no input
fn classify_prompt_source(arg: Option<&str>) -> PromptSource<'_> {
    match arg {
        Some("-") => PromptSource::Stdin,
        Some(value) => {
            let path = Path::new(value);
            if path.exists() && path.is_file() {
                PromptSource::File(path)
            } else {
                PromptSource::Inline(value)
            }
        }
        None => PromptSource::None,
    }
}

/// Read content from a prompt source.
///
/// This is the imperative shell that performs actual I/O based on the source type.
fn read_from_source(
    source: PromptSource<'_>,
    default: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    match source {
        PromptSource::Stdin => {
            use std::io::Read;
            let mut content = String::new();
            std::io::stdin().read_to_string(&mut content)?;
            Ok(content)
        }
        PromptSource::File(path) => Ok(std::fs::read_to_string(path)?),
        PromptSource::Inline(value) => Ok(value.to_string()),
        PromptSource::None => Ok(default.unwrap_or("").to_string()),
    }
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
    let source = classify_prompt_source(prompt_arg);
    let template = read_from_source(source, Some(defaults::PROMPT_TEMPLATE))?;

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
    let source = classify_prompt_source(additional_arg);
    read_from_source(source, None)
}

/// Resolve prompt for the ask command.
///
/// # Prompt Resolution
/// 1. If argument is "-", read from stdin
/// 2. If argument is a file path, read from file
/// 3. If argument is any other string, use as inline prompt
/// 4. If argument is None and stdin is piped (non-terminal), read from stdin
/// 5. If argument is None and stdin is a terminal, error (no prompt provided)
///
/// After resolution, validates that the prompt is not empty.
fn resolve_ask_prompt(prompt_arg: Option<&str>) -> Result<String, Box<dyn std::error::Error>> {
    use std::io::IsTerminal;

    let prompt = match prompt_arg {
        // Explicit argument provided - use classify_prompt_source
        Some(_) => {
            let source = classify_prompt_source(prompt_arg);
            read_from_source(source, None)?
        }
        // No argument - check for piped stdin
        None => {
            if std::io::stdin().is_terminal() {
                // Interactive terminal with no prompt - error
                return Err("No prompt provided. Usage: ralph ask 'your prompt' or echo 'your prompt' | ralph ask".into());
            }
            // Non-interactive (piped) - read from stdin
            read_from_source(PromptSource::Stdin, None)?
        }
    };

    // Validate non-empty
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return Err("Prompt cannot be empty. Provide a prompt as an argument or via stdin.".into());
    }

    Ok(prompt)
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

    // Resolve summarize prompt using shared helper
    let source = classify_prompt_source(args.summarize_prompt.as_deref());
    let prompt = read_from_source(source, Some(defaults::SUMMARIZE_PROMPT))?;

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
#[path = "main_tests.rs"]
mod tests;
