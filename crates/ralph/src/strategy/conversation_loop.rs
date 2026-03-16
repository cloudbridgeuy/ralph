//! ConversationLoop strategy implementation.
//!
//! Human-driven conversation loop where the human writes in `$EDITOR`,
//! a persona responds, and the loop repeats. Supports orchestration
//! directives between personas and human-in-the-loop directives.

use crate::highlight::ThemeConfig;
use crate::human;
use crate::invoke::{self, InvocationResult};
use crate::iteration::extract_response_text;
use crate::orchestrator::{self, Budget, OrchestrationConfig};
use crate::recovery::{invoke_with_failure_recovery, InvocationConfig, RecoveryError};
use crate::session::{finalize_session, initialize_session};
use crate::signal;
use crate::stream_processor::VerboseToolsConfig;
use crate::subprocess::StreamingSubprocessResult;
use crate::warn::warn_if_err;
use ralph_core::directive::{aggregate_responses, Directive};
use ralph_core::strategy::{KeyAction, StrategyConfig, StrategyResult};
use ralph_core::transcript::{
    build_persona_prompt, check_exit, HumanResponse, LoopAction, Speaker, TranscriptEntry,
};
use std::path::{Path, PathBuf};

use super::traits::{Strategy, StrategyExecutionContext};

/// ConversationLoop strategy: human-driven conversation with agent collaboration.
pub struct ConversationLoopStrategy;

impl Strategy for ConversationLoopStrategy {
    fn execute(
        &self,
        ctx: &StrategyExecutionContext,
    ) -> Result<StrategyResult, Box<dyn std::error::Error>> {
        execute_conversation_loop(ctx)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Loop configuration (pure construction)
// ─────────────────────────────────────────────────────────────────────────────

/// Read-only config for the conversation loop.
struct ConversationLoopConfig {
    project_path: PathBuf,
    primary_persona: String,
    available_personas: Vec<String>,
    timeout_secs: u64,
    max_attempts: usize,
    theme_config: Option<ThemeConfig>,
    verbose_tools_config: VerboseToolsConfig,
}

/// Build loop config from strategy configuration.
fn build_loop_config(config: &StrategyConfig, project_path: &Path) -> ConversationLoopConfig {
    let theme_config = Some(ThemeConfig::from_config_and_env());

    ConversationLoopConfig {
        project_path: project_path.to_path_buf(),
        primary_persona: config.primary_persona.clone(),
        available_personas: config.available_personas.clone(),
        timeout_secs: 600,
        max_attempts: 3,
        theme_config,
        verbose_tools_config: VerboseToolsConfig::default(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Metrics accumulator
// ─────────────────────────────────────────────────────────────────────────────

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

// ─────────────────────────────────────────────────────────────────────────────
// Mutable loop state
// ─────────────────────────────────────────────────────────────────────────────

struct LoopState {
    iterations_completed: usize,
    completion_reason: Option<String>,
    metrics: AccumulatedMetrics,
    key_action: Option<KeyAction>,
    transcript: Vec<TranscriptEntry>,
}

impl LoopState {
    fn new() -> Self {
        Self {
            iterations_completed: 0,
            completion_reason: None,
            metrics: AccumulatedMetrics::default(),
            key_action: None,
            transcript: Vec::new(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Main loop (imperative shell)
// ─────────────────────────────────────────────────────────────────────────────

fn execute_conversation_loop(
    ctx: &StrategyExecutionContext,
) -> Result<StrategyResult, Box<dyn std::error::Error>> {
    let loop_config = build_loop_config(&ctx.config, &ctx.project_path);

    // Initialize session
    let (session_slug, _sess_dir) = initialize_session(
        None,
        &ctx.project_path,
        None,
        Some(&loop_config.primary_persona),
    )?;

    eprintln!(
        "Starting conversation loop with persona '{}'",
        loop_config.primary_persona
    );
    eprintln!("Session: {session_slug}");
    eprintln!("Save an empty response to exit.\n");

    let mut state = LoopState::new();
    let max_iterations = ctx.max_iterations.unwrap_or(usize::MAX);

    loop {
        if signal::is_interrupted() {
            state.key_action = Some(KeyAction::SoftStop);
            state.completion_reason = Some("Interrupted".to_string());
            break;
        }

        if state.iterations_completed >= max_iterations {
            state.completion_reason = Some("Max iterations reached".to_string());
            break;
        }

        // Open editor for human input
        let human_response = human::open_editor_for_human(&state.transcript)
            .map_err(|e| -> Box<dyn std::error::Error> { Box::new(e) })?;

        if check_exit(&human_response) == LoopAction::Exit {
            state.completion_reason = Some("Human exited".to_string());
            break;
        }

        let human_text = match human_response {
            HumanResponse::Content(text) => text,
            HumanResponse::Abort => break,
        };

        // Append human entry to transcript
        state.transcript.push(TranscriptEntry {
            speaker: Speaker::Human,
            content: human_text.clone(),
        });

        // Build persona prompt with conversation history
        let prompt =
            build_persona_prompt(&state.transcript[..state.transcript.len() - 1], &human_text);
        let command = invoke::build_command(
            &prompt,
            invoke::DEFAULT_PERMISSION_MODE,
            Some(&loop_config.primary_persona),
        );

        // Invoke persona
        let invocation_config = InvocationConfig {
            command: &command,
            max_attempts: loop_config.max_attempts,
            timeout_secs: loop_config.timeout_secs,
            iteration: state.iterations_completed + 1,
            theme_config: loop_config.theme_config.as_ref(),
            session_elapsed_ms: state.metrics.elapsed_ms(),
            verbose_tools_config: &loop_config.verbose_tools_config,
            session_slug: &session_slug,
            max_iterations,
        };

        let recovery_outcome = match invoke_with_failure_recovery(&invocation_config) {
            Ok(outcome) => outcome,
            Err(RecoveryError::HardStop { .. }) => {
                state.completion_reason = Some("Hard stop".to_string());
                state.key_action = Some(KeyAction::HardStop);
                break;
            }
            Err(RecoveryError::Interrupted { .. }) => {
                state.completion_reason = Some("Interrupted".to_string());
                break;
            }
            Err(e) => return Err(e.into()),
        };

        let subprocess_result = recovery_outcome.subprocess_result;
        state.metrics.add_from_result(&subprocess_result);

        // Extract response text
        let response_text = extract_response_text(&subprocess_result.stream_result.output_blocks)
            .unwrap_or_default();

        // Handle orchestration directives
        let final_response = handle_directives(
            &response_text,
            &session_slug,
            &loop_config,
            &subprocess_result,
            &state,
        );

        // Append persona response to transcript
        state.transcript.push(TranscriptEntry {
            speaker: Speaker::Persona(loop_config.primary_persona.clone()),
            content: final_response,
        });

        state.iterations_completed += 1;

        // Check keyboard controls
        if matches!(
            recovery_outcome.key_action,
            Some(crate::keyboard::RunKeyAction::SoftStop)
        ) {
            state.completion_reason = Some("Soft stop".to_string());
            state.key_action = Some(KeyAction::SoftStop);
            break;
        }
    }

    // Finalize session
    warn_if_err(
        finalize_session(
            &session_slug,
            state.iterations_completed as u32,
            ralph_core::session::SessionOutcome::Completed,
        ),
        "Failed to finalize session",
    );

    Ok(StrategyResult {
        key_action: state.key_action,
        slug: session_slug,
        iterations_completed: state.iterations_completed,
        completion_reason: state.completion_reason,
        final_pending_stories: 0,
        total_cost_usd: state.metrics.total_cost_usd,
        total_duration_ms: state.metrics.total_duration_ms,
        total_input_tokens: state.metrics.total_input_tokens,
        total_output_tokens: state.metrics.total_output_tokens,
    })
}

/// Handle directives in persona output, including human-targeted ones.
///
/// Flow:
/// 1. Parse raw directives from response text
/// 2. Extract comments (handled separately, don't go through validation)
/// 3. Display human-targeted comments via terminal soft-block
/// 4. Partition remaining directives into human-targeted asks and persona-targeted
/// 5. Handle human asks via editor
/// 6. Handle persona directives via existing orchestration
/// 7. If human responded, continue the persona session with the aggregated response
///
/// Returns the final response text to display/record in the transcript.
fn handle_directives(
    response_text: &str,
    session_slug: &str,
    loop_config: &ConversationLoopConfig,
    subprocess_result: &StreamingSubprocessResult,
    state: &LoopState,
) -> String {
    use ralph_core::directive::{extract_comments, parse_directives, validate_directive_set};
    use ralph_core::human::partition_directives as partition_by_target;
    use ralph_core::transcript::CommentResponse;

    // Step 1: Parse raw directives
    let all_directives = parse_directives(response_text);
    if all_directives.is_empty() {
        return response_text.to_string();
    }

    // Step 2: Extract comments before validation
    let (comments, remaining) = extract_comments(all_directives);

    // Step 3: Display human-targeted comments
    for comment in &comments {
        if comment.target == "human" {
            match human::display_comment_and_wait(&comment.payload) {
                Ok(CommentResponse::Reply(text)) => {
                    // Reply to comment — treat as a response to feed back
                    crate::warn::warn(format!("Comment reply noted: {text}"));
                }
                Ok(CommentResponse::Continue) => {}
                Err(e) => {
                    crate::warn::warn(format!("Comment display failed: {e}"));
                }
            }
        }
    }

    // Step 4: Partition remaining directives by target
    let (human_refs, persona_refs) = partition_by_target(&remaining);

    let mut responses: Vec<(String, String)> = Vec::new();

    // Step 5: Handle human-targeted asks
    for directive in &human_refs {
        match human::open_editor_for_ask(&directive.payload) {
            Ok(HumanResponse::Content(text)) => {
                responses.push(("human".to_string(), text));
            }
            Ok(HumanResponse::Abort) => {
                responses.push(("human".to_string(), "(no response)".to_string()));
            }
            Err(e) => {
                crate::warn::warn(format!("Human ask failed: {e}"));
                responses.push(("human".to_string(), "(editor error)".to_string()));
            }
        }
    }

    // Step 6: Handle persona-targeted directives via existing orchestration
    let persona_directives: Vec<Directive> = persona_refs.into_iter().cloned().collect();
    if !persona_directives.is_empty() {
        // Build InvocationResult for orchestrator
        let usage = subprocess_result.stream_result.costs.usage.as_ref();
        let invocation_result = InvocationResult {
            slug: session_slug.to_string(),
            iteration_count: (state.iterations_completed + 1) as u32,
            exit_code: subprocess_result.exit_code,
            cost_usd: subprocess_result.stream_result.costs.cost_usd,
            duration_ms: subprocess_result.stream_result.costs.duration_ms,
            input_tokens: usage.map(|u| u.input_tokens),
            output_tokens: usage.map(|u| u.output_tokens),
            response_text: Some(response_text.to_string()),
            persona: Some(loop_config.primary_persona.clone()),
        };

        let orch_config = build_orchestration_config(loop_config);

        if let Ok(validated) = validate_directive_set(persona_directives) {
            warn_if_err(
                orchestrator::orchestrate(&invocation_result, validated, &orch_config)
                    .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string())),
                "Orchestration failed",
            );
        }
    }

    // Step 7: Continue persona session with human responses
    if !responses.is_empty() {
        let response_refs: Vec<(&str, &str)> = responses
            .iter()
            .map(|(n, t)| (n.as_str(), t.as_str()))
            .collect();
        let aggregated = aggregate_responses(&response_refs);

        let orch_config = build_orchestration_config(loop_config);

        match orchestrator::continue_session(
            session_slug,
            &loop_config.primary_persona,
            &aggregated,
            None,
            &orch_config,
        ) {
            Ok(result) => {
                return result
                    .response_text
                    .unwrap_or_else(|| response_text.to_string());
            }
            Err(e) => {
                crate::warn::warn(format!(
                    "Failed to continue session with human response: {e}"
                ));
            }
        }
    }

    response_text.to_string()
}

/// Build an OrchestrationConfig from the conversation loop config.
fn build_orchestration_config(loop_config: &ConversationLoopConfig) -> OrchestrationConfig {
    OrchestrationConfig {
        project_path: loop_config.project_path.clone(),
        timeout_secs: loop_config.timeout_secs,
        theme_config: loop_config
            .theme_config
            .clone()
            .unwrap_or_else(ThemeConfig::from_config_and_env),
        verbose_tools: loop_config.verbose_tools_config.clone(),
        budget: Budget::new(orchestrator::DEFAULT_BUDGET),
        known_personas: loop_config.available_personas.clone(),
    }
}

#[cfg(test)]
mod tests {
    use ralph_core::directive::{extract_comments, DirectiveVerb};

    use super::*;

    #[test]
    fn extract_comments_separates_comment_directives() {
        let directives = vec![
            Directive {
                verb: DirectiveVerb::Comment,
                target: "human".to_string(),
                payload: "Looking good so far".to_string(),
            },
            Directive {
                verb: DirectiveVerb::Ask,
                target: "editor-agent".to_string(),
                payload: "Check pacing".to_string(),
            },
        ];
        let (comments, remaining) = extract_comments(directives);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].verb, DirectiveVerb::Comment);
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].verb, DirectiveVerb::Ask);
    }

    #[test]
    fn extract_comments_no_comments() {
        let directives = vec![Directive {
            verb: DirectiveVerb::Ask,
            target: "reviewer".to_string(),
            payload: "Review".to_string(),
        }];
        let (comments, remaining) = extract_comments(directives);
        assert!(comments.is_empty());
        assert_eq!(remaining.len(), 1);
    }

    #[test]
    fn build_orchestration_config_sets_fields() {
        let config = ConversationLoopConfig {
            project_path: std::path::PathBuf::from("/tmp/test"),
            primary_persona: "storyteller".to_string(),
            available_personas: vec!["editor-agent".to_string()],
            timeout_secs: 600,
            max_attempts: 3,
            theme_config: None,
            verbose_tools_config: VerboseToolsConfig::default(),
        };
        let orch = build_orchestration_config(&config);
        assert_eq!(orch.project_path, std::path::PathBuf::from("/tmp/test"));
        assert_eq!(orch.timeout_secs, 600);
        assert_eq!(orch.known_personas, vec!["editor-agent".to_string()]);
    }
}
