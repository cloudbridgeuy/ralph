//! Run command execution logic.
//!
//! This module contains the orchestration logic for executing the run command,
//! including failure recovery prompting, resume handling, and error processing.

use crate::cli::RunArgs;
use crate::highlight;
use crate::iteration;
use crate::prompt::{prompt_on_failure, FailureAction};
use crate::prompt_source::{classify_prompt_source, read_from_source};
use crate::session;
use crate::signal;
use crate::startup;
use crate::stream_processor::VerboseToolsConfig;
use crate::warn::{warn, warn_if_err};
use ralph_core::context::{defaults, resolve_prd_path, substitute_template_placeholders};
use ralph_core::session::{PausedState, SessionOutcome};
use std::path::Path;

use super::{run, RunConfig, RunError};

use crate::subprocess::StreamingSubprocessResult;

/// Write a partial iteration log for an interrupted or hard-stopped iteration.
///
/// This helper extracts the common pattern of building an IterationLog from
/// partial subprocess results and writing it to the session directory.
///
/// # Arguments
///
/// * `session_slug` - Session identifier for determining the log directory
/// * `iteration` - The 1-based iteration number
/// * `partial` - The partial subprocess result containing whatever output was captured
/// * `pending` - Number of pending stories at iteration start
/// * `message_prefix` - Prefix for the success message (e.g., "" or "\n")
fn write_partial_iteration_log(
    session_slug: &str,
    iteration: usize,
    partial: &StreamingSubprocessResult,
    pending: usize,
    message_prefix: &str,
) {
    let session_dir = session::session_dir(session_slug);

    let partial_log = iteration::IterationLog {
        sequence: iteration as u32,
        started_at: chrono::Utc::now(),
        completed_at: chrono::Utc::now(),
        exit_code: partial.exit_code,
        pending_before: pending,
        pending_after: pending, // Same as before since iteration was interrupted
        prompt: None,           // Run command doesn't track prompt per iteration
        response: iteration::extract_response_text(&partial.stream_result.output_blocks),
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

    match iteration::write_iteration_log(&session_dir, &partial_log) {
        Ok(_) => eprintln!("{}Partial iteration {} saved.", message_prefix, iteration),
        Err(e) => warn(format!("Failed to write partial iteration log: {e}")),
    }
}

/// Config for run command execution.
pub struct RunExecutionConfig {
    pub project_root: std::path::PathBuf,
    pub prd_path: std::path::PathBuf,
    pub command: String,
    pub prompt: String,
    pub completion_marker: String,
}

/// Context for handling subprocess failure recovery.
struct FailureRecoveryContext {
    summary: String,
    session_slug: String,
    iterations_completed: usize,
    total_iterations_completed: usize,
}

/// Result of failure recovery handling.
enum FailureRecoveryResult {
    Retry { updated_total_iterations: usize },
    Aborted(Box<dyn std::error::Error>),
}

/// Handle subprocess failure with user prompting and session finalization.
fn handle_failure_recovery(
    ctx: &FailureRecoveryContext,
    current_session_slug: &mut Option<String>,
) -> FailureRecoveryResult {
    if current_session_slug.is_none() {
        *current_session_slug = Some(ctx.session_slug.clone());
    }

    match prompt_on_failure(&ctx.summary) {
        Some(FailureAction::Retry) => {
            let updated = ctx.total_iterations_completed + ctx.iterations_completed;
            eprintln!("\nContinuing run (session '{}')...\n", ctx.session_slug);
            FailureRecoveryResult::Retry {
                updated_total_iterations: updated,
            }
        }
        Some(FailureAction::Abort) => {
            let final_iterations = ctx.total_iterations_completed + ctx.iterations_completed;
            warn_if_err(
                session::finalize_session(
                    &ctx.session_slug,
                    final_iterations as u32,
                    SessionOutcome::Aborted,
                ),
                "Failed to finalize session",
            );
            FailureRecoveryResult::Aborted("Aborted by user".into())
        }
        None => {
            let final_iterations = ctx.total_iterations_completed + ctx.iterations_completed;
            warn_if_err(
                session::finalize_session(
                    &ctx.session_slug,
                    final_iterations as u32,
                    SessionOutcome::Failed,
                ),
                "Failed to finalize session",
            );
            eprintln!("Non-interactive mode - aborting.");
            FailureRecoveryResult::Aborted(ctx.summary.clone().into())
        }
    }
}

/// Execute the run command.
pub fn execute_run(args: RunArgs) -> Result<(), Box<dyn std::error::Error>> {
    warn_if_err(signal::init(), "Failed to initialize signal handler");

    let project_root = std::env::current_dir()?;

    if args.resume {
        return execute_run_resume(args, project_root);
    }

    let prd_path = resolve_prd_path(&project_root, args.prd.as_deref());

    let command_template = args
        .command
        .clone()
        .unwrap_or_else(|| defaults::COMMAND_TEMPLATE.to_string());

    let completion_marker = args
        .completion_marker
        .clone()
        .unwrap_or_else(|| defaults::COMPLETION_MARKER.to_string());

    let additional_prompt = resolve_additional_prompt(args.additional_prompt.as_deref())?;

    let prompt = resolve_prompt(
        args.prompt.as_deref(),
        &prd_path,
        &completion_marker,
        &additional_prompt,
    )?;

    let command = substitute_prompt_in_command(&command_template, &prompt);

    let exec_config = RunExecutionConfig {
        project_root,
        prd_path,
        command,
        prompt,
        completion_marker,
    };
    execute_run_with_prompting(args, exec_config)
}

/// Execute run command in resume mode.
fn execute_run_resume(
    args: RunArgs,
    project_root: std::path::PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let paused_state = session::load_paused_state_for_project(&project_root)?;

    eprintln!(
        "Resuming session '{}' from iteration {}...\n",
        paused_state.slug,
        paused_state.iterations_completed + 1
    );

    let prd_path = paused_state.prd_path.clone();

    let command_template = args
        .command
        .clone()
        .unwrap_or_else(|| defaults::COMMAND_TEMPLATE.to_string());

    let completion_marker = args
        .completion_marker
        .clone()
        .unwrap_or_else(|| defaults::COMPLETION_MARKER.to_string());

    let additional_prompt = resolve_additional_prompt(args.additional_prompt.as_deref())?;

    let prompt = resolve_prompt(
        args.prompt.as_deref(),
        &prd_path,
        &completion_marker,
        &additional_prompt,
    )?;

    let command = substitute_prompt_in_command(&command_template, &prompt);

    let exec_config = RunExecutionConfig {
        project_root,
        prd_path,
        command,
        prompt,
        completion_marker,
    };

    let mut resumed_args = args;
    resumed_args.slug = Some(paused_state.slug);

    warn_if_err(
        session::delete_paused_state(),
        "Failed to delete paused state file",
    );

    execute_run_with_prompting_internal(
        resumed_args,
        exec_config,
        paused_state.iterations_completed as usize,
    )
}

/// Execute run loop with interactive failure recovery prompting.
fn execute_run_with_prompting(
    args: RunArgs,
    exec_config: RunExecutionConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    execute_run_with_prompting_internal(args, exec_config, 0)
}

/// Internal implementation of run loop with configurable starting iteration.
fn execute_run_with_prompting_internal(
    args: RunArgs,
    exec_config: RunExecutionConfig,
    initial_iterations_completed: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut current_session_slug: Option<String> = None;
    let mut total_iterations_completed: usize = initial_iterations_completed;

    let theme_config = Some(
        highlight::ThemeConfig::from_config_and_env()
            .merge_cli(args.theme.as_deref(), args.no_background),
    );

    let custom_prd_path = args.prd.clone();
    let custom_command = args.command.is_some();
    let custom_prompt = args.prompt.is_some();
    let custom_completion_marker = args.completion_marker.is_some();
    let custom_additional_prompt = args.additional_prompt.is_some();

    let verbose_tools_config = VerboseToolsConfig::from_arg(args.verbose_tools.as_deref());
    for warning in verbose_tools_config.warnings() {
        warn(warning);
    }

    loop {
        let config = RunConfig {
            max_iterations: args.iterations,
            slug: current_session_slug.clone().or_else(|| args.slug.clone()),
            command: exec_config.command.clone(),
            prompt: exec_config.prompt.clone(),
            completion_marker: exec_config.completion_marker.clone(),
            prd_path: exec_config.prd_path.clone(),
            max_attempts: args.max_attempts,
            starting_iteration: total_iterations_completed,
            timeout_secs: args.timeout,
            theme_config: theme_config.clone(),
            custom_prd_path: custom_prd_path.clone(),
            custom_command,
            custom_prompt,
            custom_completion_marker,
            custom_additional_prompt,
            verbose_tools_config: verbose_tools_config.clone(),
            show_prompt: !args.no_prompt,
        };

        match run(config) {
            Ok(result) => {
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
                session_slug,
                iterations_completed,
                ..
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
                session_slug,
                iterations_completed,
                ..
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
                if let (Some(partial), Some(pending)) = (partial_result, pending_before) {
                    let iteration = total_iterations_completed + iterations_completed + 1;
                    write_partial_iteration_log(&session_slug, iteration, &partial, pending, "");
                }

                let final_iterations = total_iterations_completed + iterations_completed;
                warn_if_err(
                    session::finalize_session(
                        &session_slug,
                        final_iterations as u32,
                        SessionOutcome::Interrupted,
                    ),
                    "Failed to finalize session",
                );
                eprintln!("Interrupted. Session '{}' saved.", session_slug);
                return Err("Interrupted by signal".into());
            }
            Err(RunError::HardStop {
                session_slug,
                iterations_completed,
                partial_result,
                pending_before,
                prd_path,
            }) => {
                if let (Some(partial), Some(pending)) = (partial_result, pending_before) {
                    let iteration = total_iterations_completed + iterations_completed + 1;
                    write_partial_iteration_log(&session_slug, iteration, &partial, pending, "\n");
                }

                let final_iterations = total_iterations_completed + iterations_completed;
                let paused_state = PausedState::new(
                    session_slug.clone(),
                    exec_config.project_root.clone(),
                    final_iterations as u32,
                    prd_path,
                );
                warn_if_err(
                    session::save_paused_state(&paused_state),
                    "Failed to save paused state",
                );

                warn_if_err(
                    session::finalize_session(
                        &session_slug,
                        final_iterations as u32,
                        SessionOutcome::Interrupted,
                    ),
                    "Failed to finalize session",
                );

                eprintln!(
                    "Hard stop. Session '{}' paused after {} iteration(s).",
                    session_slug, final_iterations
                );
                eprintln!("Resume with: ralph run --resume");
                return Err("Hard-stopped by user".into());
            }
            Err(e) => {
                return Err(e.into());
            }
        }
    }
}

/// Resolve the prompt from various sources.
fn resolve_prompt(
    prompt_arg: Option<&str>,
    prd_path: &Path,
    completion_marker: &str,
    additional_prompt: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let source = classify_prompt_source(prompt_arg);
    let template = read_from_source(source, Some(defaults::PROMPT_TEMPLATE))?;

    Ok(substitute_template_placeholders(
        &template,
        prd_path,
        completion_marker,
        additional_prompt,
    ))
}

/// Resolve additional prompt from various sources.
fn resolve_additional_prompt(
    additional_arg: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    let source = classify_prompt_source(additional_arg);
    read_from_source(source, None)
}

/// Substitute `{prompt}` placeholder in command template.
fn substitute_prompt_in_command(template: &str, prompt: &str) -> String {
    let escaped = prompt.replace('\'', "'\"'\"'");
    let quoted_prompt = format!("'{}'", escaped);
    template.replace("{prompt}", &quoted_prompt)
}
