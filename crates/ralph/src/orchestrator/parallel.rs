//! Parallel invocation engine for multi-agent directive execution.
//!
//! Uses `std::thread::scope` to invoke all targets concurrently while
//! allowing shared borrows of the orchestration config.

use ralph_core::directive::Directive;

use super::{display, OrchestrationConfig, OrchestrationError};
use crate::invoke::{self, InvocationConfig, InvocationResult};

/// Invoke all directives in parallel using scoped threads.
///
/// Each thread checks the budget before invoking. If the budget is exhausted,
/// that thread returns `Err(BudgetExhausted)`. Results are collected in order
/// matching the input directives.
///
/// Routing status is printed for each directive before spawning threads.
pub fn parallel_invoke(
    directives: &[Directive],
    originator_name: &str,
    config: &OrchestrationConfig,
) -> Vec<Result<InvocationResult, OrchestrationError>> {
    // Print routing status for all directives up front (sequential, before spawning)
    for directive in directives {
        display::print_routing_status(
            originator_name,
            &directive.verb,
            &directive.target,
            &directive.payload,
            &config.budget,
        );
        display::print_persona_banner(&directive.target);
    }

    std::thread::scope(|s| {
        let handles: Vec<_> = directives
            .iter()
            .map(|directive| s.spawn(|| invoke_directive(directive, originator_name, config)))
            .collect();

        handles
            .into_iter()
            .map(|handle| {
                // join() returns Result<T, Any>; a panic in the thread produces Err.
                // Re-panic so the caller sees the original panic rather than silently
                // swallowing it.  This matches the behaviour of sequential execution.
                match handle.join() {
                    Ok(result) => result,
                    Err(panic_payload) => std::panic::resume_unwind(panic_payload),
                }
            })
            .collect()
    })
}

/// Invoke a single directive after checking the budget.
///
/// Returns `Err(BudgetExhausted)` if no budget remains.
fn invoke_directive(
    directive: &Directive,
    originator_name: &str,
    config: &OrchestrationConfig,
) -> Result<InvocationResult, OrchestrationError> {
    if !config.budget.try_consume() {
        return Err(OrchestrationError::BudgetExhausted);
    }

    let invocation_config = InvocationConfig {
        prompt: directive.payload.clone(),
        timeout_secs: config.timeout_secs,
        theme_config: config.theme_config.clone(),
        verbose_tools: config.verbose_tools.clone(),
        project_path: config.project_path.clone(),
        slug: None,
        continuation: None,
        clone: None,
        permission_mode: invoke::DEFAULT_PERMISSION_MODE.to_string(),
        persona: Some(directive.target.clone()),
        on_behalf_of: Some(originator_name.to_string()),
    };

    Ok(invoke::invoke(invocation_config)?)
}
