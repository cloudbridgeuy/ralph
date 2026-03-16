//! Strategy execution dispatcher.
//!
//! Loads and validates the named strategy, resolves its kind to a typed
//! enum variant, and dispatches to the corresponding implementation module
//! via the `Strategy` trait. Uses static dispatch (enum match + generics)
//! — no dynamic dispatch or plugin loading.

use crate::cli::StrategyExecuteArgs;
use crate::signal;
use crate::startup;
use crate::warn::warn_if_err;
use ralph_core::strategy::{StrategyKind, StrategyResult};

use super::conversation_loop::ConversationLoopStrategy;
use super::prd_loop::PrdLoopStrategy;
use super::traits::{run_strategy, StrategyExecutionContext};
use super::{find_strategy_by_name, load_all_strategies, LoadedStrategy};

/// Execute a named strategy.
///
/// 1. Load and validate all strategies for the project
/// 2. Find the requested strategy by name
/// 3. Dispatch on its resolved `StrategyKind` to a `Strategy` implementation
pub fn execute_strategy_execute(
    args: StrategyExecuteArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    warn_if_err(signal::init(), "Failed to initialize signal handler");

    let project_path = std::env::current_dir()?;
    let strategies = load_all_strategies(&project_path)?;
    let matched = find_strategy_by_name(&strategies, &args.name)?;

    let ctx = build_execution_context(matched, &args, &project_path);

    let result = match matched.kind {
        StrategyKind::PrdLoop => run_strategy(&PrdLoopStrategy, ctx)?,
        StrategyKind::ConversationLoop => run_strategy(&ConversationLoopStrategy, ctx)?,
    };

    display_strategy_result(&result);
    Ok(())
}

/// Build a `StrategyExecutionContext` from CLI args and loaded strategy.
fn build_execution_context(
    strategy: &LoadedStrategy,
    args: &StrategyExecuteArgs,
    project_path: &std::path::Path,
) -> StrategyExecutionContext {
    StrategyExecutionContext {
        config: strategy.config.clone(),
        project_path: project_path.to_path_buf(),
        max_iterations: args.max_iterations,
        resume: args.resume,
    }
}

/// Display strategy result as a run summary.
fn display_strategy_result(result: &StrategyResult) {
    let summary = startup::RunSummary {
        slug: result.slug.clone(),
        iterations_completed: result.iterations_completed,
        completion_reason: result.completion_reason.clone(),
        total_cost_usd: result.total_cost_usd,
        total_duration_ms: result.total_duration_ms,
        total_input_tokens: result.total_input_tokens,
        total_output_tokens: result.total_output_tokens,
        final_pending_stories: result.final_pending_stories,
    };
    startup::display_run_summary(&summary);
}
