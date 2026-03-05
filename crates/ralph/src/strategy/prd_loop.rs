//! PrdLoop strategy implementation.
//!
//! Translates a `prd-loop` strategy configuration into a `RunConfig`
//! and delegates to the existing `run::run()` iteration loop.
//! The primary difference from `ralph run` is that the command is
//! built with `--agent` for the strategy's primary persona.

use crate::highlight;
use crate::invoke;
use crate::run::RunConfig;
use crate::stream_processor::VerboseToolsConfig;
use ralph_core::context::{defaults, resolve_prd_path, substitute_template_placeholders};
use ralph_core::strategy::StrategyConfig;
use std::path::Path;

/// Configuration for executing a prd-loop strategy.
pub(crate) struct PrdLoopParams<'a> {
    pub strategy: &'a StrategyConfig,
    pub project_path: &'a Path,
    pub max_iterations: Option<usize>,
    pub resume_iteration: usize,
    pub slug: Option<String>,
}

/// Build a `RunConfig` for a prd-loop strategy execution.
///
/// Constructs config from inputs. Reads theme configuration from environment.
pub(crate) fn build_run_config(
    params: &PrdLoopParams,
) -> Result<RunConfig, Box<dyn std::error::Error>> {
    let prd_path = resolve_prd_path(params.project_path, None);

    let completion_marker = defaults::COMPLETION_MARKER.to_string();

    // Build prompt from template + prompt_aggregates
    let additional_prompt = build_additional_prompt(&params.strategy.prompt_aggregates);
    let template = defaults::PROMPT_TEMPLATE;
    let prompt = substitute_template_placeholders(
        template,
        &prd_path,
        &completion_marker,
        &additional_prompt,
    );

    // Build command using persona invocation (--agent flag)
    let command = invoke::build_command(
        &prompt,
        invoke::DEFAULT_PERMISSION_MODE,
        Some(&params.strategy.primary_persona),
    );

    let theme_config = Some(highlight::ThemeConfig::from_config_and_env());

    Ok(RunConfig {
        max_iterations: params.max_iterations,
        slug: params.slug.clone(),
        command,
        prompt,
        completion_marker,
        prd_path,
        max_attempts: 3,
        starting_iteration: params.resume_iteration,
        timeout_secs: 600,
        theme_config,
        custom_prd_path: None,
        custom_command: false,
        custom_prompt: false,
        custom_completion_marker: false,
        custom_additional_prompt: !params.strategy.prompt_aggregates.is_empty(),
        verbose_tools_config: VerboseToolsConfig::default(),
        show_prompt: true,
    })
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
}
