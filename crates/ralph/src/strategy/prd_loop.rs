//! PrdLoop strategy implementation.
//!
//! Owns the iteration loop for PRD-driven development. Each iteration
//! invokes the LLM subprocess, writes logs, checks completion, and
//! optionally scans for orchestration directives between iterations.
//!
//! This module follows the Functional Core - Imperative Shell pattern:
//! pure helper functions for config building and type mapping, with the
//! `execute` method as the imperative shell orchestrating I/O.

use crate::git::capture_and_write_diff;
use crate::highlight::ThemeConfig;
use crate::init::verify_prd_exists;
use crate::invoke::{self, InvocationResult};
use crate::iteration::{
    extract_response_text, write_iteration_log, Chunk, IterationLog, LogMetadata, LogToolCall,
};
use crate::keyboard::RunKeyAction;
use crate::orchestrator::{self, Budget, OrchestrationConfig};
use crate::recovery::{invoke_with_failure_recovery, InvocationConfig, RecoveryError};
use crate::session::{
    delete_paused_state, finalize_session, initialize_session, load_paused_state_for_project,
    save_paused_state,
};
use crate::signal;
use crate::startup::{
    display_iteration_header, display_iteration_summary, display_prompt, display_startup_info,
    AttachedFile, IterationHeader, IterationSummary, PromptDisplay, StartupInfo,
};
use crate::stream_processor::VerboseToolsConfig;
use crate::subprocess::StreamingSubprocessResult;
use crate::warn::warn_if_err;
use ralph_core::completion::{check_completion, CompletionReason};
use ralph_core::directive::ValidatedDirectiveSet;
use ralph_core::prd::{count_pending_stories, has_prd_changed, parse_prd};
use ralph_core::session::{PausedState, SessionOutcome};
use ralph_core::strategy::{KeyAction, StrategyConfig, StrategyResult};
use std::fs;
use std::path::{Path, PathBuf};

use super::traits::{Strategy, StrategyExecutionContext};

/// PrdLoop strategy: iterates through PRD stories using a persona.
///
/// Owns the full iteration loop with orchestration support between
/// iterations. Scans subprocess output for directives and resolves
/// them via the orchestrator before continuing.
pub struct PrdLoopStrategy;

impl Strategy for PrdLoopStrategy {
    fn execute(
        &self,
        ctx: &StrategyExecutionContext,
    ) -> Result<StrategyResult, Box<dyn std::error::Error>> {
        execute_prd_loop(ctx)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Iteration loop (imperative shell)
// ─────────────────────────────────────────────────────────────────────────────

/// Configuration for the prd-loop strategy's iteration loop.
///
/// Groups all read-only parameters needed throughout the loop to keep
/// function signatures under 5 arguments.
struct StrategyInvocationConfig {
    command: String,
    prompt: String,
    completion_marker: String,
    prd_path: PathBuf,
    project_path: PathBuf,
    max_attempts: usize,
    timeout_secs: u64,
    theme_config: Option<ThemeConfig>,
    verbose_tools_config: VerboseToolsConfig,
    available_personas: Vec<String>,
    custom_additional_prompt: bool,
    primary_persona: String,
}

/// Build a `StrategyInvocationConfig` from strategy configuration and project path.
///
/// Pure construction except for reading theme configuration from env.
fn build_loop_config(config: &StrategyConfig, project_path: &Path) -> StrategyInvocationConfig {
    let prd_path = ralph_core::context::resolve_prd_path(project_path, None);
    let completion_marker = ralph_core::context::defaults::COMPLETION_MARKER.to_string();
    let additional_prompt = build_additional_prompt(&config.prompt_aggregates);
    let prompt = ralph_core::context::substitute_template_placeholders(
        ralph_core::context::defaults::PROMPT_TEMPLATE,
        &prd_path,
        &completion_marker,
        &additional_prompt,
    );
    let command = invoke::build_command(
        &prompt,
        invoke::DEFAULT_PERMISSION_MODE,
        Some(&config.primary_persona),
    );
    let theme_config = Some(ThemeConfig::from_config_and_env());

    StrategyInvocationConfig {
        command,
        prompt,
        completion_marker,
        prd_path,
        project_path: project_path.to_path_buf(),
        max_attempts: 3,
        timeout_secs: 600,
        theme_config,
        verbose_tools_config: VerboseToolsConfig::default(),
        available_personas: config.available_personas.clone(),
        custom_additional_prompt: !config.prompt_aggregates.is_empty(),
        primary_persona: config.primary_persona.clone(),
    }
}

/// Accumulated metrics across iterations.
#[derive(Debug, Default)]
struct AccumulatedMetrics {
    total_cost_usd: Option<f64>,
    total_duration_ms: Option<u64>,
    total_input_tokens: Option<u64>,
    total_output_tokens: Option<u64>,
}

impl AccumulatedMetrics {
    fn add_from_result(&mut self, result: &StreamingSubprocessResult) {
        if let Some(cost) = result.stream_result.costs.cost_usd {
            self.total_cost_usd = Some(self.total_cost_usd.unwrap_or(0.0) + cost);
        }
        if let Some(duration) = result.stream_result.costs.duration_ms {
            self.total_duration_ms = Some(self.total_duration_ms.unwrap_or(0) + duration);
        }
        if let Some(ref usage) = result.stream_result.costs.usage {
            self.total_input_tokens =
                Some(self.total_input_tokens.unwrap_or(0) + usage.input_tokens);
            self.total_output_tokens =
                Some(self.total_output_tokens.unwrap_or(0) + usage.output_tokens);
        }
    }

    fn elapsed_ms(&self) -> u64 {
        self.total_duration_ms.unwrap_or(0)
    }
}

/// Mutable state maintained across iterations.
struct IterationState {
    iterations_completed: usize,
    completion_reason: Option<String>,
    final_pending_stories: usize,
    metrics: AccumulatedMetrics,
    key_action: Option<KeyAction>,
}

impl IterationState {
    fn new(initial_pending: usize) -> Self {
        Self {
            iterations_completed: 0,
            completion_reason: None,
            final_pending_stories: initial_pending,
            metrics: AccumulatedMetrics::default(),
            key_action: None,
        }
    }
}

/// Execute the full prd-loop strategy lifecycle.
fn execute_prd_loop(
    ctx: &StrategyExecutionContext,
) -> Result<StrategyResult, Box<dyn std::error::Error>> {
    let loop_config = build_loop_config(&ctx.config, &ctx.project_path);
    let identity = StrategyIdentity {
        strategy_name: &ctx.config.name,
        persona: &loop_config.primary_persona,
    };

    // Handle --resume
    let (starting_iteration, slug_override) = if ctx.resume {
        let paused = load_paused_state_for_project(&ctx.project_path)?;
        let slug = paused.slug.clone();
        let iterations = paused.iterations_completed as usize;
        eprintln!(
            "Resuming session '{}' from iteration {}...\n",
            slug,
            iterations + 1
        );
        warn_if_err(delete_paused_state(), "Failed to delete paused state file");
        (iterations, Some(slug))
    } else {
        (0, None)
    };

    // Pre-loop: verify PRD and parse stories
    verify_prd_exists(&loop_config.prd_path)?;
    let prd_content = read_prd_file(&loop_config.prd_path)?;
    let prd_analysis = parse_prd(&prd_content)?;

    if prd_analysis.pending_count == 0 {
        return Err("All stories complete. Nothing to do.".into());
    }

    let max_iterations = ctx.max_iterations.unwrap_or(prd_analysis.pending_count);

    // Initialize session
    let (session_slug, sess_dir) = initialize_session(
        slug_override.as_deref(),
        &ctx.project_path,
        Some(loop_config.prompt.clone()),
        None,
    )?;

    // Display startup info (only on fresh start)
    if starting_iteration == 0 {
        display_strategy_startup(
            &loop_config,
            &session_slug,
            &sess_dir,
            &prd_analysis,
            max_iterations,
        );
    }

    // Iteration loop
    let mut state = IterationState::new(prd_analysis.pending_count);
    let remaining_iterations = max_iterations.saturating_sub(starting_iteration);

    for relative_iteration in 1..=remaining_iterations {
        if signal::is_interrupted() {
            state.key_action = Some(KeyAction::SoftStop);
            state.completion_reason = Some("Interrupted".to_string());
            break;
        }

        let iteration = starting_iteration + relative_iteration;

        // Re-read PRD and count pending
        let prd_before = read_prd_file(&loop_config.prd_path)?;
        let pending_before = count_pending_stories(&prd_before)?;

        if pending_before == 0 {
            state.completion_reason = Some(format!("{:?}", CompletionReason::AllStoriesComplete));
            state.iterations_completed = relative_iteration.saturating_sub(1);
            break;
        }

        // Display iteration header
        let header = IterationHeader {
            iteration,
            max_iterations: Some(max_iterations),
            pending_stories: pending_before,
        };
        display_iteration_header(&header);

        // Snapshot PRD for change detection
        let prd_snapshot = prd_before;

        // Invoke subprocess with failure recovery
        let invocation_config = build_invocation_config(
            &loop_config,
            &session_slug,
            iteration,
            max_iterations,
            state.metrics.elapsed_ms(),
        );

        let recovery_outcome = match invoke_with_failure_recovery(&invocation_config) {
            Ok(outcome) => outcome,
            Err(RecoveryError::HardStop { partial_result }) => {
                // Save paused state for later resume
                let final_iterations = starting_iteration + state.iterations_completed;
                let paused = PausedState::new(
                    session_slug.clone(),
                    loop_config.project_path.clone(),
                    final_iterations as u32,
                    loop_config.prd_path.clone(),
                );
                warn_if_err(save_paused_state(&paused), "Failed to save paused state");
                warn_if_err(
                    finalize_session(
                        &session_slug,
                        final_iterations as u32,
                        SessionOutcome::Interrupted,
                    ),
                    "Failed to finalize session",
                );

                // Write partial iteration log if we have data
                if let Some(partial) = partial_result {
                    write_partial_log(&sess_dir, iteration, &partial, pending_before, &identity);
                }

                eprintln!(
                    "Hard stop. Session '{}' paused after {} iteration(s).",
                    session_slug, final_iterations
                );
                eprintln!("Resume with: ralph strategy execute <name> --resume");
                return Err("Hard-stopped by user".into());
            }
            Err(RecoveryError::Interrupted { partial_result }) => {
                let final_iterations = starting_iteration + state.iterations_completed;
                if let Some(partial) = partial_result {
                    write_partial_log(&sess_dir, iteration, &partial, pending_before, &identity);
                }
                warn_if_err(
                    finalize_session(
                        &session_slug,
                        final_iterations as u32,
                        SessionOutcome::Interrupted,
                    ),
                    "Failed to finalize session",
                );
                eprintln!("Interrupted. Session '{}' saved.", session_slug);
                return Err("Interrupted by signal".into());
            }
            Err(e) => return Err(e.into()),
        };

        let subprocess_result = recovery_outcome.subprocess_result;
        let key_action = recovery_outcome.key_action;

        // Write iteration log
        let mut iteration_log =
            build_iteration_log(iteration, pending_before, &subprocess_result, &identity);
        write_iteration_log(&sess_dir, &iteration_log)?;

        // Check interrupt after log write
        if signal::is_interrupted() {
            state.iterations_completed = relative_iteration;
            state.completion_reason = Some("Interrupted".to_string());
            break;
        }

        // Capture git diff
        let diff_path = sess_dir.join(format!("iteration-{iteration}.diff"));
        warn_if_err(
            capture_and_write_diff(&diff_path),
            "Failed to capture git diff",
        );

        // Display summary and accumulate metrics
        display_iteration_metrics(iteration, &subprocess_result);
        state.metrics.add_from_result(&subprocess_result);

        // Re-read PRD and check completion
        let prd_after = read_prd_file(&loop_config.prd_path)?;
        let pending_after = count_pending_stories(&prd_after)?;
        state.final_pending_stories = pending_after;

        // Update iteration log with pending_after
        iteration_log.pending_after = pending_after;
        write_iteration_log(&sess_dir, &iteration_log)?;

        // Check completion
        if let Some(reason) = check_completion(
            pending_after,
            &subprocess_result.stream_result.raw_text,
            &loop_config.completion_marker,
        ) {
            state.completion_reason = Some(format!("{reason:?}"));
            state.iterations_completed = relative_iteration;
            break;
        }

        // Check stuck state
        if !has_prd_changed(&prd_snapshot, &prd_after) {
            warn_if_err(
                finalize_session(
                    &session_slug,
                    (starting_iteration + relative_iteration) as u32,
                    SessionOutcome::Failed,
                ),
                "Failed to finalize session",
            );
            return Err("PRD unchanged after iteration. LLM may be stuck.".into());
        }

        state.iterations_completed = relative_iteration;

        // Handle keyboard controls
        if matches!(key_action, Some(RunKeyAction::SoftStop)) {
            eprintln!("\nSoft stop requested. Finishing after this iteration.");
            state.completion_reason = Some(format!("{:?}", CompletionReason::SoftStop));
            state.key_action = Some(KeyAction::SoftStop);
            break;
        }

        // Orchestration between iterations: scan for directives
        let response_text = extract_response_text(&subprocess_result.stream_result.output_blocks);
        if let Some(text) = response_text {
            let invocation_result = build_invocation_result_for_orchestrator(
                &session_slug,
                iteration,
                &subprocess_result,
                Some(text),
                &loop_config.primary_persona,
            );
            if let Some(directives) = orchestrator::scan_for_directives(
                &invocation_result,
                &loop_config.available_personas,
            ) {
                // Filter directives to only allowed personas
                let filtered =
                    filter_directives_by_persona(directives, &loop_config.available_personas);
                if let Some(filtered_directives) = filtered {
                    let orch_config = OrchestrationConfig {
                        project_path: loop_config.project_path.clone(),
                        timeout_secs: loop_config.timeout_secs,
                        theme_config: loop_config
                            .theme_config
                            .clone()
                            .unwrap_or_else(ThemeConfig::from_config_and_env),
                        verbose_tools: loop_config.verbose_tools_config.clone(),
                        budget: Budget::new(orchestrator::DEFAULT_BUDGET),
                        known_personas: loop_config.available_personas.clone(),
                    };
                    warn_if_err(
                        orchestrator::orchestrate(
                            &invocation_result,
                            filtered_directives,
                            &orch_config,
                        )
                        .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string())),
                        "Orchestration failed",
                    );
                }
            }
        }
    }

    // Finalize session
    let total_iterations = starting_iteration + state.iterations_completed;
    warn_if_err(
        finalize_session(
            &session_slug,
            total_iterations as u32,
            SessionOutcome::Completed,
        ),
        "Failed to finalize session",
    );

    Ok(StrategyResult {
        key_action: state.key_action,
        slug: session_slug,
        iterations_completed: state.iterations_completed,
        completion_reason: state.completion_reason,
        final_pending_stories: state.final_pending_stories,
        total_cost_usd: state.metrics.total_cost_usd,
        total_duration_ms: state.metrics.total_duration_ms,
        total_input_tokens: state.metrics.total_input_tokens,
        total_output_tokens: state.metrics.total_output_tokens,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Pure helpers (functional core)
// ─────────────────────────────────────────────────────────────────────────────

/// Filter directives to only include those targeting allowed personas.
///
/// Warns about and removes directives targeting personas not in the
/// `available_personas` list. Returns `None` if all directives are filtered out.
fn filter_directives_by_persona(
    directives: ValidatedDirectiveSet,
    available_personas: &[String],
) -> Option<ValidatedDirectiveSet> {
    let filter =
        |ds: Vec<ralph_core::directive::Directive>| -> Vec<ralph_core::directive::Directive> {
            ds.into_iter()
                .filter(|d| {
                    let allowed = available_personas.iter().any(|p| p == &d.target);
                    if !allowed {
                        eprintln!(
                            "Warning: directive targets unlisted persona '{}', skipping",
                            d.target
                        );
                    }
                    allowed
                })
                .collect()
        };

    match directives {
        ValidatedDirectiveSet::Asks(asks) => {
            let filtered = filter(asks);
            if filtered.is_empty() {
                None
            } else {
                Some(ValidatedDirectiveSet::Asks(filtered))
            }
        }
        ValidatedDirectiveSet::Handovers(handovers) => {
            let filtered = filter(handovers);
            if filtered.is_empty() {
                None
            } else {
                Some(ValidatedDirectiveSet::Handovers(filtered))
            }
        }
    }
}

/// Join prompt_aggregates into a single additional prompt string.
///
/// Pure function — no I/O.
fn build_additional_prompt(aggregates: &[String]) -> String {
    if aggregates.is_empty() {
        return String::new();
    }
    aggregates.join("\n")
}

/// Build an `InvocationConfig` for subprocess invocation.
///
/// Pure construction from loop config and iteration parameters.
fn build_invocation_config<'a>(
    loop_config: &'a StrategyInvocationConfig,
    session_slug: &'a str,
    iteration: usize,
    max_iterations: usize,
    session_elapsed_ms: u64,
) -> InvocationConfig<'a> {
    InvocationConfig {
        command: &loop_config.command,
        max_attempts: loop_config.max_attempts,
        timeout_secs: loop_config.timeout_secs,
        iteration,
        theme_config: loop_config.theme_config.as_ref(),
        session_elapsed_ms,
        verbose_tools_config: &loop_config.verbose_tools_config,
        session_slug,
        max_iterations,
    }
}

/// Build an `InvocationResult` from subprocess output for the orchestrator.
///
/// The orchestrator API operates on `InvocationResult` rather than
/// `StreamingSubprocessResult`. This adapter maps the fields needed
/// for directive scanning and orchestration.
fn build_invocation_result_for_orchestrator(
    session_slug: &str,
    iteration: usize,
    subprocess_result: &StreamingSubprocessResult,
    response_text: Option<String>,
    persona: &str,
) -> InvocationResult {
    let usage = subprocess_result.stream_result.costs.usage.as_ref();
    InvocationResult {
        slug: session_slug.to_string(),
        iteration_count: iteration as u32,
        exit_code: subprocess_result.exit_code,
        cost_usd: subprocess_result.stream_result.costs.cost_usd,
        duration_ms: subprocess_result.stream_result.costs.duration_ms,
        input_tokens: usage.map(|u| u.input_tokens),
        output_tokens: usage.map(|u| u.output_tokens),
        response_text,
        persona: Some(persona.to_string()),
    }
}

/// Build an iteration log from subprocess result.
///
/// Pure function — constructs log data from result fields.
/// Includes strategy name and persona for strategy-driven iterations.
fn build_iteration_log(
    iteration: usize,
    pending_before: usize,
    result: &StreamingSubprocessResult,
    identity: &StrategyIdentity,
) -> IterationLog {
    let metadata = LogMetadata::from_extracted(
        result.stream_result.metadata.clone(),
        result.stream_result.costs.clone(),
    );
    let tool_calls = LogToolCall::from_interactions(&result.stream_result.tool_interactions);
    let chunks = Chunk::from_parsed_chunks(&result.stream_result.chunks);
    let response = extract_response_text(&result.stream_result.output_blocks);

    IterationLog {
        sequence: iteration as u32,
        started_at: chrono::Utc::now(),
        completed_at: chrono::Utc::now(),
        exit_code: result.exit_code,
        pending_before,
        pending_after: 0,
        prompt: None,
        response,
        metadata: metadata.into_option(),
        tool_calls,
        chunks,
        output_blocks: result.stream_result.output_blocks.clone(),
        strategy_name: Some(identity.strategy_name.to_string()),
        persona: Some(identity.persona.to_string()),
    }
}

/// Display iteration summary with cost, duration, and token usage.
fn display_iteration_metrics(iteration: usize, result: &StreamingSubprocessResult) {
    let usage = result.stream_result.costs.usage.as_ref();
    let summary = IterationSummary {
        iteration,
        cost_usd: result.stream_result.costs.cost_usd,
        duration_ms: result.stream_result.costs.duration_ms,
        model: result.stream_result.metadata.model.clone(),
        input_tokens: usage.map(|u| u.input_tokens),
        output_tokens: usage.map(|u| u.output_tokens),
    };
    display_iteration_summary(&summary);
}

/// Display startup information for the strategy.
fn display_strategy_startup(
    config: &StrategyInvocationConfig,
    session_slug: &str,
    sess_dir: &Path,
    prd_analysis: &ralph_core::prd::PrdAnalysis,
    max_iterations: usize,
) {
    let startup_info = StartupInfo {
        slug: session_slug.to_string(),
        total_stories: prd_analysis.total_stories,
        pending_stories: prd_analysis.pending_count,
        completed_stories: prd_analysis.completed_count,
        max_iterations,
        iterations_from_arg: false,
        custom_prd_path: None,
        custom_command: false,
        custom_prompt: false,
        custom_completion_marker: false,
        custom_additional_prompt: config.custom_additional_prompt,
        session_dir: sess_dir.to_path_buf(),
    };
    display_startup_info(&startup_info);

    let attached_files = vec![AttachedFile::new(config.prd_path.clone())];
    let prompt_display = PromptDisplay {
        prompt: &config.prompt,
        attached_files,
    };
    display_prompt(&prompt_display);
}

/// Read PRD file content.
fn read_prd_file(path: &Path) -> Result<String, Box<dyn std::error::Error>> {
    fs::read_to_string(path)
        .map_err(|e| format!("Failed to read PRD file at {}: {e}", path.display()).into())
}

/// Context identifying the strategy and persona for log entries.
///
/// Groups identity fields shared across all iterations to keep
/// function signatures under 5 arguments.
struct StrategyIdentity<'a> {
    strategy_name: &'a str,
    persona: &'a str,
}

/// Write a partial iteration log for interrupted/hard-stopped iterations.
fn write_partial_log(
    sess_dir: &Path,
    iteration: usize,
    partial: &StreamingSubprocessResult,
    pending_before: usize,
    identity: &StrategyIdentity,
) {
    let log = build_iteration_log(iteration, pending_before, partial, identity);
    warn_if_err(
        write_iteration_log(sess_dir, &log),
        "Failed to write partial iteration log",
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_additional_prompt_empty() {
        assert_eq!(build_additional_prompt(&[]), "");
    }

    #[test]
    fn test_build_additional_prompt_single() {
        let agg = vec!["Run tests first".to_string()];
        assert_eq!(build_additional_prompt(&agg), "Run tests first");
    }

    #[test]
    fn test_build_additional_prompt_multiple() {
        let agg = vec![
            "Run tests first".to_string(),
            "Use conventional commits".to_string(),
        ];
        assert_eq!(
            build_additional_prompt(&agg),
            "Run tests first\nUse conventional commits"
        );
    }

    #[test]
    fn test_build_invocation_config_fields() {
        let config = StrategyInvocationConfig {
            command: "test-cmd".to_string(),
            prompt: "test prompt".to_string(),
            completion_marker: "DONE".to_string(),
            prd_path: PathBuf::from("/tmp/prd.toml"),
            project_path: PathBuf::from("/tmp/project"),
            max_attempts: 3,
            timeout_secs: 600,
            theme_config: None,
            verbose_tools_config: VerboseToolsConfig::default(),
            available_personas: vec!["reviewer".to_string()],
            custom_additional_prompt: false,
            primary_persona: "developer".to_string(),
        };

        let inv = build_invocation_config(&config, "test-slug", 2, 5, 1000);
        assert_eq!(inv.command, "test-cmd");
        assert_eq!(inv.max_attempts, 3);
        assert_eq!(inv.timeout_secs, 600);
        assert_eq!(inv.iteration, 2);
        assert!(inv.theme_config.is_none());
        assert_eq!(inv.session_elapsed_ms, 1000);
        assert_eq!(inv.session_slug, "test-slug");
        assert_eq!(inv.max_iterations, 5);
    }

    #[test]
    fn test_build_invocation_result_for_orchestrator() {
        use crate::stream_processor::StreamProcessorResult;

        let stream_result = StreamProcessorResult::default();
        let subprocess_result = StreamingSubprocessResult {
            exit_code: 0,
            stderr: String::new(),
            stream_result,
        };

        let result = build_invocation_result_for_orchestrator(
            "slug-1",
            3,
            &subprocess_result,
            Some("response text".to_string()),
            "developer",
        );

        assert_eq!(result.slug, "slug-1");
        assert_eq!(result.iteration_count, 3);
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.response_text, Some("response text".to_string()));
        assert_eq!(result.persona, Some("developer".to_string()));
    }

    #[test]
    fn test_build_loop_config_command_uses_persona_agent() {
        let strategy_config = StrategyConfig {
            name: "test".to_string(),
            description: "Test strategy".to_string(),
            kind: "prd-loop".to_string(),
            primary_persona: "developer".to_string(),
            available_personas: vec![],
            prompt_aggregates: vec![],
        };

        let config = build_loop_config(&strategy_config, Path::new("/tmp/project"));
        assert!(
            config.command.contains("--agent developer"),
            "command should invoke primary_persona's agent file: {}",
            config.command
        );
    }

    #[test]
    fn test_build_loop_config_sets_fields() {
        let strategy_config = StrategyConfig {
            name: "test".to_string(),
            description: "Test strategy".to_string(),
            kind: "prd-loop".to_string(),
            primary_persona: "developer".to_string(),
            available_personas: vec!["reviewer".to_string(), "tester".to_string()],
            prompt_aggregates: vec!["Run tests".to_string()],
        };

        let config = build_loop_config(&strategy_config, Path::new("/tmp/project"));
        assert_eq!(config.primary_persona, "developer");
        assert_eq!(
            config.available_personas,
            vec!["reviewer".to_string(), "tester".to_string()]
        );
        assert!(config.custom_additional_prompt);
        assert_eq!(config.max_attempts, 3);
        assert_eq!(config.timeout_secs, 600);
    }

    #[test]
    fn test_build_loop_config_no_aggregates() {
        let strategy_config = StrategyConfig {
            name: "test".to_string(),
            description: "Test".to_string(),
            kind: "prd-loop".to_string(),
            primary_persona: "dev".to_string(),
            available_personas: vec![],
            prompt_aggregates: vec![],
        };

        let config = build_loop_config(&strategy_config, Path::new("/tmp"));
        assert!(!config.custom_additional_prompt);
    }

    #[test]
    fn test_build_iteration_log_sets_strategy_and_persona() {
        use crate::stream_processor::StreamProcessorResult;

        let stream_result = StreamProcessorResult::default();
        let subprocess_result = StreamingSubprocessResult {
            exit_code: 0,
            stderr: String::new(),
            stream_result,
        };

        let identity = StrategyIdentity {
            strategy_name: "prd-loop",
            persona: "developer",
        };
        let log = build_iteration_log(1, 3, &subprocess_result, &identity);
        assert_eq!(log.strategy_name, Some("prd-loop".to_string()));
        assert_eq!(log.persona, Some("developer".to_string()));
        assert_eq!(log.sequence, 1);
        assert_eq!(log.pending_before, 3);
        assert_eq!(log.pending_after, 0);
    }

    // ── filter_directives_by_persona tests ──────────────────────────────

    fn make_directive(target: &str) -> ralph_core::directive::Directive {
        ralph_core::directive::Directive {
            verb: ralph_core::directive::DirectiveVerb::Ask,
            target: target.to_string(),
            payload: format!("msg for {target}"),
        }
    }

    fn make_handover(target: &str) -> ralph_core::directive::Directive {
        ralph_core::directive::Directive {
            verb: ralph_core::directive::DirectiveVerb::Handover,
            target: target.to_string(),
            payload: format!("handover to {target}"),
        }
    }

    #[test]
    fn filter_keeps_allowed_personas() {
        let directives =
            ValidatedDirectiveSet::Asks(vec![make_directive("reviewer"), make_directive("tester")]);
        let allowed = vec!["reviewer".to_string(), "tester".to_string()];
        let result = filter_directives_by_persona(directives, &allowed);
        match result {
            Some(ValidatedDirectiveSet::Asks(asks)) => assert_eq!(asks.len(), 2),
            other => panic!("expected Asks with 2, got {other:?}"),
        }
    }

    #[test]
    fn filter_removes_unlisted_persona() {
        let directives = ValidatedDirectiveSet::Asks(vec![
            make_directive("reviewer"),
            make_directive("unknown"),
        ]);
        let allowed = vec!["reviewer".to_string()];
        let result = filter_directives_by_persona(directives, &allowed);
        match result {
            Some(ValidatedDirectiveSet::Asks(asks)) => {
                assert_eq!(asks.len(), 1);
                assert_eq!(asks[0].target, "reviewer");
            }
            other => panic!("expected Asks with 1, got {other:?}"),
        }
    }

    #[test]
    fn filter_returns_none_when_all_unlisted() {
        let directives = ValidatedDirectiveSet::Asks(vec![
            make_directive("unknown1"),
            make_directive("unknown2"),
        ]);
        let allowed = vec!["reviewer".to_string()];
        assert!(filter_directives_by_persona(directives, &allowed).is_none());
    }

    #[test]
    fn filter_works_for_handovers() {
        let directives = ValidatedDirectiveSet::Handovers(vec![
            make_handover("deployer"),
            make_handover("reviewer"),
        ]);
        let allowed = vec!["reviewer".to_string()];
        let result = filter_directives_by_persona(directives, &allowed);
        match result {
            Some(ValidatedDirectiveSet::Handovers(hs)) => {
                assert_eq!(hs.len(), 1);
                assert_eq!(hs[0].target, "reviewer");
            }
            other => panic!("expected Handovers with 1, got {other:?}"),
        }
    }

    #[test]
    fn filter_empty_available_removes_all() {
        let directives = ValidatedDirectiveSet::Asks(vec![make_directive("reviewer")]);
        assert!(filter_directives_by_persona(directives, &[]).is_none());
    }
}
