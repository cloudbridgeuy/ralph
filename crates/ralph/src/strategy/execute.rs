//! Strategy execution dispatcher.
//!
//! Loads and validates the named strategy, resolves its kind to a typed
//! enum variant, and dispatches to the corresponding implementation module.

use crate::cli::StrategyExecuteArgs;
use crate::run;
use crate::signal;
use crate::startup;
use crate::warn::warn_if_err;
use ralph_core::strategy::StrategyKind;

use super::prd_loop;
use super::{find_strategy_by_name, load_all_strategies};

/// Execute a named strategy.
///
/// 1. Load and validate all strategies for the project
/// 2. Find the requested strategy by name
/// 3. Dispatch on its resolved `StrategyKind`
pub fn execute_strategy_execute(
    args: StrategyExecuteArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    warn_if_err(signal::init(), "Failed to initialize signal handler");

    let project_path = std::env::current_dir()?;
    let strategies = load_all_strategies(&project_path)?;
    let matched = find_strategy_by_name(&strategies, &args.name)?;

    match matched.kind {
        StrategyKind::PrdLoop => execute_prd_loop(matched, args, &project_path),
    }
}

/// Execute the prd-loop strategy kind.
fn execute_prd_loop(
    strategy: &super::LoadedStrategy,
    args: StrategyExecuteArgs,
    project_path: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if args.resume {
        eprintln!("Warning: --resume is not yet implemented for strategy execute. Starting from iteration 0.");
    }

    let params = prd_loop::PrdLoopParams {
        strategy: &strategy.config,
        project_path,
        max_iterations: args.max_iterations,
        resume_iteration: 0, // TODO: wire --resume in a follow-up
        slug: None,
    };

    let config = prd_loop::build_run_config(&params)?;

    match run::run(config) {
        Ok(result) => {
            let summary = startup::RunSummary {
                slug: result.slug,
                iterations_completed: result.iterations_completed,
                completion_reason: result.completion_reason.map(|r| format!("{:?}", r)),
                total_cost_usd: result.total_cost_usd,
                total_duration_ms: result.total_duration_ms,
                total_input_tokens: result.total_input_tokens,
                total_output_tokens: result.total_output_tokens,
                final_pending_stories: result.final_pending_stories,
            };
            startup::display_run_summary(&summary);
            Ok(())
        }
        Err(run::RunError::NoPendingStories) => {
            eprintln!("All stories complete. Nothing to do.");
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}
