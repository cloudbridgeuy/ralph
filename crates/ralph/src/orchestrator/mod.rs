//! Orchestrator module for multi-agent directive routing.
//!
//! After a persona finishes, the orchestrator scans its output for directives,
//! validates them, and executes handovers (or asks) to target personas.
//! This is the imperative shell for multi-agent orchestration.

mod ask;
mod conversation;
mod display;
mod parallel;

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use ralph_core::directive::{
    parse_directives, validate_directive_set, Directive, ValidatedDirectiveSet,
};

use crate::highlight::ThemeConfig;
use crate::invoke::{self, ContinuationInfo, InvocationConfig, InvocationError, InvocationResult};
use crate::iteration::{count_iterations, IterationError};
use crate::session::{self, SessionError};
use crate::stream_processor::VerboseToolsConfig;
use crate::warn::warn;

/// Default maximum number of orchestrated invocations per session.
pub const DEFAULT_BUDGET: usize = 10;

// ─────────────────────────────────────────────────────────────────────────────
// Budget
// ─────────────────────────────────────────────────────────────────────────────

/// Thread-safe invocation budget that tracks how many invocations remain.
///
/// Used to prevent runaway orchestration loops by capping the total number
/// of sub-invocations the orchestrator can make in a single session.
#[derive(Clone)]
pub struct Budget {
    remaining: Arc<AtomicUsize>,
    limit: usize,
}

impl Budget {
    /// Create a new budget with the given limit.
    pub fn new(limit: usize) -> Self {
        Self {
            remaining: Arc::new(AtomicUsize::new(limit)),
            limit,
        }
    }

    /// Try to consume one invocation from the budget.
    ///
    /// Returns `true` if an invocation was consumed, `false` if exhausted.
    pub fn try_consume(&self) -> bool {
        // fetch_update returns Err(current) when the closure returns None
        self.remaining
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                if current > 0 {
                    Some(current - 1)
                } else {
                    None
                }
            })
            .is_ok()
    }

    /// Return the number of remaining invocations.
    pub fn remaining(&self) -> usize {
        self.remaining.load(Ordering::SeqCst)
    }

    /// Return the original budget limit.
    pub fn limit(&self) -> usize {
        self.limit
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// OrchestrationConfig
// ─────────────────────────────────────────────────────────────────────────────

/// Configuration for the orchestration engine.
pub struct OrchestrationConfig {
    /// Absolute path to the project directory.
    pub project_path: PathBuf,
    /// Timeout for each subprocess invocation in seconds.
    pub timeout_secs: u64,
    /// Theme configuration for syntax highlighting.
    pub theme_config: ThemeConfig,
    /// Verbose tools configuration.
    pub verbose_tools: VerboseToolsConfig,
    /// Invocation budget for the orchestration session.
    pub budget: Budget,
}

// ─────────────────────────────────────────────────────────────────────────────
// OrchestrationError
// ─────────────────────────────────────────────────────────────────────────────

/// Errors that can occur during orchestration.
#[derive(Debug, thiserror::Error)]
pub enum OrchestrationError {
    /// An invocation of a target persona failed.
    #[error("Invocation failed: {0}")]
    Invocation(#[from] InvocationError),
    /// The invocation budget has been exhausted.
    #[error("Budget exhausted")]
    BudgetExhausted,
    /// A session lookup or initialization failed.
    #[error("Session error: {0}")]
    Session(#[from] SessionError),
    /// Failed to count iteration files in a session directory.
    #[error("Iteration error: {0}")]
    Iteration(#[from] IterationError),
}

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

/// Scan an invocation result for directives.
///
/// Extracts the response text from the result, parses directives, and validates
/// them. Returns `None` if no directives are found or if validation fails
/// (e.g., mixed ask/handover verbs).
pub fn scan_for_directives(result: &InvocationResult) -> Option<ValidatedDirectiveSet> {
    let response_text = result.response_text.as_deref()?;

    let directives = parse_directives(response_text);
    if directives.is_empty() {
        return None;
    }

    match validate_directive_set(directives) {
        Ok(validated) => Some(validated),
        Err(e) => {
            warn(format!(
                "Directive validation failed ({}), ignoring directives",
                e
            ));
            None
        }
    }
}

/// Orchestrate execution of validated directives.
///
/// Dispatches to the appropriate executor based on the directive type:
/// - `Handovers` — invokes target personas in parallel, scans for sub-directives sequentially
/// - `Asks` — invokes targets in parallel, aggregates responses, continues originator
pub fn orchestrate(
    originator: &InvocationResult,
    directives: ValidatedDirectiveSet,
    config: &OrchestrationConfig,
) -> Result<(), OrchestrationError> {
    let originator_name = originator.display_name();

    match directives {
        ValidatedDirectiveSet::Handovers(ref handover_directives) => {
            execute_handovers(originator_name, handover_directives, config)?;
        }
        ValidatedDirectiveSet::Asks(ref ask_directives) => {
            ask::execute_asks(ask_directives, originator, config)?;
        }
    }

    display::print_orchestration_summary(&config.budget);
    Ok(())
}

/// Continue an existing session by invoking a persona with conversation history.
///
/// Looks up the session directory for the given slug, counts existing iteration
/// files to determine the next sequence number, builds a `ContinuationInfo`,
/// and delegates to `invoke()`.
///
/// This is a convenience wrapper used by the ask executor to continue the
/// originator's session after receiving responses from target personas.
pub fn continue_session(
    session_slug: &str,
    persona: &str,
    prompt: &str,
    on_behalf_of: Option<&str>,
    config: &OrchestrationConfig,
) -> Result<InvocationResult, OrchestrationError> {
    let session_dir = session::session_dir(session_slug);
    let existing_count = count_iterations(&session_dir)?;

    let continuation = ContinuationInfo {
        slug: session_slug.to_string(),
        next_sequence: existing_count + 1,
        session_dir,
    };

    let invocation_config = InvocationConfig {
        prompt: prompt.to_string(),
        timeout_secs: config.timeout_secs,
        theme_config: config.theme_config.clone(),
        verbose_tools: config.verbose_tools.clone(),
        project_path: config.project_path.clone(),
        slug: None,
        continuation: Some(continuation),
        clone: None,
        permission_mode: invoke::DEFAULT_PERMISSION_MODE.to_string(),
        persona: Some(persona.to_string()),
        on_behalf_of: on_behalf_of.map(|s| s.to_string()),
    };

    Ok(invoke::invoke(invocation_config)?)
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal
// ─────────────────────────────────────────────────────────────────────────────

/// Execute handover directives in parallel.
///
/// Invokes all targets concurrently via [`parallel::parallel_invoke`], then
/// scans each result sequentially for sub-directives and recurses.
/// Budget checks happen inside each parallel thread.
fn execute_handovers(
    originator_name: &str,
    directives: &[Directive],
    config: &OrchestrationConfig,
) -> Result<(), OrchestrationError> {
    let results = parallel::parallel_invoke(directives, originator_name, config);

    for (directive, result) in directives.iter().zip(results) {
        let result = result?;

        // Scan for sub-directives and recurse
        if let Some(sub_directives) = scan_for_directives(&result) {
            let target_name = directive.target.as_str();
            orchestrate_inner(target_name, &result, sub_directives, config)?;
        }
    }

    Ok(())
}

/// Inner orchestration for recursive sub-directive handling.
///
/// Separated from the public `orchestrate` to avoid printing the summary
/// at each recursion level — the summary is only printed once at the top.
fn orchestrate_inner(
    originator_name: &str,
    originator: &InvocationResult,
    directives: ValidatedDirectiveSet,
    config: &OrchestrationConfig,
) -> Result<(), OrchestrationError> {
    match directives {
        ValidatedDirectiveSet::Handovers(ref handover_directives) => {
            execute_handovers(originator_name, handover_directives, config)?;
        }
        ValidatedDirectiveSet::Asks(ref ask_directives) => {
            ask::execute_asks(ask_directives, originator, config)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Budget tests
    // =========================================================================

    #[test]
    fn budget_new_sets_remaining_and_limit() {
        let budget = Budget::new(5);
        assert_eq!(budget.remaining(), 5);
        assert_eq!(budget.limit(), 5);
    }

    #[test]
    fn budget_try_consume_decrements() {
        let budget = Budget::new(3);
        assert!(budget.try_consume());
        assert_eq!(budget.remaining(), 2);
        assert!(budget.try_consume());
        assert_eq!(budget.remaining(), 1);
        assert!(budget.try_consume());
        assert_eq!(budget.remaining(), 0);
    }

    #[test]
    fn budget_try_consume_returns_false_when_exhausted() {
        let budget = Budget::new(1);
        assert!(budget.try_consume());
        assert!(!budget.try_consume());
        assert_eq!(budget.remaining(), 0);
    }

    #[test]
    fn budget_zero_limit() {
        let budget = Budget::new(0);
        assert_eq!(budget.remaining(), 0);
        assert_eq!(budget.limit(), 0);
        assert!(!budget.try_consume());
    }

    #[test]
    fn budget_clone_shares_state() {
        let budget = Budget::new(5);
        let cloned = budget.clone();
        assert!(budget.try_consume());
        assert_eq!(cloned.remaining(), 4);
    }

    // =========================================================================
    // scan_for_directives tests
    // =========================================================================

    #[test]
    fn scan_no_response_text_returns_none() {
        let result = InvocationResult {
            slug: "test".to_string(),
            iteration_count: 1,
            exit_code: 0,
            cost_usd: None,
            duration_ms: None,
            input_tokens: None,
            output_tokens: None,
            response_text: None,
            persona: None,
        };
        assert!(scan_for_directives(&result).is_none());
    }

    #[test]
    fn scan_empty_response_returns_none() {
        let result = InvocationResult {
            slug: "test".to_string(),
            iteration_count: 1,
            exit_code: 0,
            cost_usd: None,
            duration_ms: None,
            input_tokens: None,
            output_tokens: None,
            response_text: Some("No directives here.".to_string()),
            persona: None,
        };
        assert!(scan_for_directives(&result).is_none());
    }

    #[test]
    fn scan_finds_handover_directive() {
        let result = InvocationResult {
            slug: "test".to_string(),
            iteration_count: 1,
            exit_code: 0,
            cost_usd: None,
            duration_ms: None,
            input_tokens: None,
            output_tokens: None,
            response_text: Some(
                r#"<ralph-handover to="deployer">Deploy to prod.</ralph-handover>"#.to_string(),
            ),
            persona: None,
        };
        let directives = scan_for_directives(&result);
        assert!(directives.is_some());
        assert!(matches!(
            directives,
            Some(ValidatedDirectiveSet::Handovers(_))
        ));
    }

    #[test]
    fn scan_mixed_verbs_returns_none() {
        let result = InvocationResult {
            slug: "test".to_string(),
            iteration_count: 1,
            exit_code: 0,
            cost_usd: None,
            duration_ms: None,
            input_tokens: None,
            output_tokens: None,
            response_text: Some(
                r#"<ralph-ask to="a">question</ralph-ask><ralph-handover to="b">task</ralph-handover>"#
                    .to_string(),
            ),
            persona: None,
        };
        assert!(scan_for_directives(&result).is_none());
    }
}
