//! Strategy trait and execution context.
//!
//! Defines the interface that all strategy implementations must satisfy.
//! Dispatch is via enum matching on `StrategyKind` — no dynamic dispatch
//! or plugin loading. All strategies are compiled into the binary.

use ralph_core::strategy::{IterationDecision, StrategyConfig, StrategyResult};
use std::path::PathBuf;

/// Context for strategy execution.
///
/// Bundles the strategy configuration with CLI arguments and project
/// metadata needed to execute a strategy. Passed to `Strategy::execute`.
pub struct StrategyExecutionContext {
    /// Parsed and validated strategy configuration from TOML.
    pub config: StrategyConfig,
    /// Project root directory.
    pub project_path: PathBuf,
    /// Maximum iterations (from `--max-iterations` CLI flag).
    pub max_iterations: Option<usize>,
    /// Whether to resume a previously stopped session.
    pub resume: bool,
}

/// Trait for strategy implementations.
///
/// Each strategy kind implements this trait. The `execute` method runs the
/// entire strategy (which may involve multiple internal iterations), and
/// `between_iterations` provides a hook for inter-iteration decisions
/// such as orchestration directive resolution.
///
/// # Dispatch
///
/// Strategies are dispatched via exhaustive `match` on [`StrategyKind`],
/// which creates the concrete implementation and calls its methods.
/// This avoids dynamic dispatch while keeping the interface uniform.
///
/// # Adding a New Strategy
///
/// 1. Add a variant to `StrategyKind` in `crates/core/src/strategy.rs`
/// 2. Add a case in `resolve_kind()` in the same file
/// 3. Create a struct implementing this trait in `crates/ralph/src/strategy/`
/// 4. Add a dispatch arm in `execute.rs`
pub trait Strategy {
    /// Execute the strategy.
    ///
    /// Runs the full strategy lifecycle: session initialization, iteration
    /// loop, completion detection, and session finalization. Returns a
    /// `StrategyResult` with metrics and completion state.
    ///
    /// The `key_action` field in the result communicates any keyboard
    /// control detected during execution (e.g., soft stop, hard stop).
    fn execute(
        &self,
        ctx: &StrategyExecutionContext,
    ) -> Result<StrategyResult, Box<dyn std::error::Error>>;

    /// Called between iterations to decide what to do next.
    ///
    /// Receives the result of the most recent iteration and returns a
    /// decision: continue to the next iteration, resolve orchestration
    /// directives first, or stop.
    ///
    /// The default implementation always continues. Strategy implementations
    /// override this to add orchestration support (Story 4).
    #[allow(dead_code)] // Part of trait API for external loop drivers; PrdLoop handles orchestration inline
    fn between_iterations(&self, _result: &StrategyResult) -> IterationDecision {
        IterationDecision::Continue
    }
}

/// Run a strategy with result display.
///
/// Generic over `S: Strategy` to maintain static dispatch. Executes the
/// strategy and maps the result for the caller.
pub fn run_strategy<S: Strategy>(
    strategy: &S,
    ctx: StrategyExecutionContext,
) -> Result<StrategyResult, Box<dyn std::error::Error>> {
    strategy.execute(&ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::strategy::KeyAction;

    /// A minimal test strategy that returns a fixed result.
    struct TestStrategy {
        result: StrategyResult,
    }

    impl Strategy for TestStrategy {
        fn execute(
            &self,
            _ctx: &StrategyExecutionContext,
        ) -> Result<StrategyResult, Box<dyn std::error::Error>> {
            Ok(StrategyResult {
                key_action: self.result.key_action,
                slug: self.result.slug.clone(),
                iterations_completed: self.result.iterations_completed,
                completion_reason: self.result.completion_reason.clone(),
                final_pending_stories: self.result.final_pending_stories,
                total_cost_usd: self.result.total_cost_usd,
                total_duration_ms: self.result.total_duration_ms,
                total_input_tokens: self.result.total_input_tokens,
                total_output_tokens: self.result.total_output_tokens,
            })
        }
    }

    fn make_ctx() -> StrategyExecutionContext {
        StrategyExecutionContext {
            config: StrategyConfig {
                name: "test".to_string(),
                description: "Test strategy".to_string(),
                kind: "prd-loop".to_string(),
                primary_persona: "dev".to_string(),
                available_personas: vec![],
                prompt_aggregates: vec![],
            },
            project_path: PathBuf::from("/tmp/test"),
            max_iterations: None,
            resume: false,
        }
    }

    fn make_result() -> StrategyResult {
        StrategyResult {
            key_action: None,
            slug: "test-slug".to_string(),
            iterations_completed: 3,
            completion_reason: Some("AllStoriesComplete".to_string()),
            final_pending_stories: 0,
            total_cost_usd: Some(0.05),
            total_duration_ms: Some(30000),
            total_input_tokens: Some(1000),
            total_output_tokens: Some(2000),
        }
    }

    #[test]
    fn test_strategy_execute_returns_result() {
        let strategy = TestStrategy {
            result: make_result(),
        };
        let ctx = make_ctx();
        let result = strategy.execute(&ctx).unwrap();
        assert_eq!(result.slug, "test-slug");
        assert_eq!(result.iterations_completed, 3);
    }

    #[test]
    fn test_strategy_between_iterations_default() {
        let strategy = TestStrategy {
            result: make_result(),
        };
        let result = make_result();
        assert_eq!(
            strategy.between_iterations(&result),
            IterationDecision::Continue
        );
    }

    #[test]
    fn test_strategy_key_action_propagated() {
        let mut result = make_result();
        result.key_action = Some(KeyAction::SoftStop);
        let strategy = TestStrategy { result };
        let ctx = make_ctx();
        let output = strategy.execute(&ctx).unwrap();
        assert_eq!(output.key_action, Some(KeyAction::SoftStop));
    }

    #[test]
    fn test_run_strategy_delegates() {
        let strategy = TestStrategy {
            result: make_result(),
        };
        let ctx = make_ctx();
        let result = run_strategy(&strategy, ctx).unwrap();
        assert_eq!(result.iterations_completed, 3);
    }

    /// Test that a custom between_iterations override works.
    struct StoppingStrategy;

    impl Strategy for StoppingStrategy {
        fn execute(
            &self,
            _ctx: &StrategyExecutionContext,
        ) -> Result<StrategyResult, Box<dyn std::error::Error>> {
            Ok(make_result())
        }

        fn between_iterations(&self, _result: &StrategyResult) -> IterationDecision {
            IterationDecision::Stop
        }
    }

    #[test]
    fn test_custom_between_iterations() {
        let strategy = StoppingStrategy;
        let result = make_result();
        assert_eq!(
            strategy.between_iterations(&result),
            IterationDecision::Stop
        );
    }
}
