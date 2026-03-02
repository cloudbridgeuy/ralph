//! Ask executor for multi-agent directive routing.
//!
//! Implements the ask round-trip: originator emits ask directives, targets are
//! invoked sequentially, responses are aggregated, and the originator is
//! continued with the aggregated prompt. Sub-directives from targets are
//! resolved recursively before returning.

use ralph_core::directive::{aggregate_responses, Directive, ValidatedDirectiveSet};

use super::{
    continue_session, display, scan_for_directives, OrchestrationConfig, OrchestrationError,
};
use crate::invoke::{self, InvocationConfig, InvocationResult};

/// Execute ask directives sequentially and continue the originator.
///
/// For each directive, invokes the target persona and calls [`resolve`] on the
/// result. After all targets have responded, aggregates the responses into a
/// single prompt and continues the originator's session via [`continue_session`].
/// If the continuation produces further directives, recurses through
/// [`super::orchestrate_inner`].
pub fn execute_asks(
    directives: &[Directive],
    originator: &InvocationResult,
    config: &OrchestrationConfig,
) -> Result<(), OrchestrationError> {
    let originator_name = originator
        .persona
        .as_deref()
        .unwrap_or(originator.slug.as_str());

    // Phase 1: invoke each target and resolve sub-directives
    let mut responses: Vec<(String, String)> = Vec::new();

    for directive in directives {
        if !config.budget.try_consume() {
            return Err(OrchestrationError::BudgetExhausted);
        }

        display::print_routing_status(
            originator_name,
            &directive.verb,
            &directive.target,
            &directive.payload,
            &config.budget,
        );

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
        };

        let result = invoke::invoke(invocation_config)?;
        let resolved_text = resolve(&result, originator_name, config)?;
        responses.push((directive.target.clone(), resolved_text));
    }

    // Phase 2: aggregate responses and continue the originator
    let response_refs: Vec<(&str, &str)> = responses
        .iter()
        .map(|(name, text)| (name.as_str(), text.as_str()))
        .collect();
    let aggregated = aggregate_responses(&response_refs);

    if !config.budget.try_consume() {
        return Err(OrchestrationError::BudgetExhausted);
    }

    let continuation_result =
        continue_session(&originator.slug, originator_name, &aggregated, config)?;

    // Phase 3: scan continuation for more directives and recurse
    if let Some(sub_directives) = scan_for_directives(&continuation_result) {
        super::orchestrate_inner(
            originator_name,
            &continuation_result,
            sub_directives,
            config,
        )?;
    }

    Ok(())
}

/// Resolve a target invocation result, handling sub-directives if present.
///
/// Three cases:
/// 1. **No sub-directives**: return the response text directly.
/// 2. **Sub-directives to new personas** (not the originator): execute them,
///    continue the target with the sub-results, and return the target's final
///    response.
/// 3. **Sub-directive back to originator**: placeholder error — conversation
///    loops are not yet implemented (Task 4.2).
fn resolve(
    result: &InvocationResult,
    originator_persona: &str,
    config: &OrchestrationConfig,
) -> Result<String, OrchestrationError> {
    let response_text = result.response_text.clone().unwrap_or_default();

    let sub_directives = match scan_for_directives(result) {
        Some(d) => d,
        None => return Ok(response_text),
    };

    // Check if any sub-directive targets the originator (conversation loop)
    let targets_originator = match &sub_directives {
        ValidatedDirectiveSet::Asks(directives) | ValidatedDirectiveSet::Handovers(directives) => {
            directives.iter().any(|d| d.target == originator_persona)
        }
    };

    if targets_originator {
        return Err(OrchestrationError::ConversationNotImplemented);
    }

    // Sub-directives to new personas — execute them
    let target_name = result.persona.as_deref().unwrap_or(result.slug.as_str());

    match sub_directives {
        ValidatedDirectiveSet::Asks(ref ask_directives) => {
            // Target emitted asks: invoke those, aggregate, continue target, return final
            let mut sub_responses: Vec<(String, String)> = Vec::new();

            for directive in ask_directives {
                if !config.budget.try_consume() {
                    return Err(OrchestrationError::BudgetExhausted);
                }

                display::print_routing_status(
                    target_name,
                    &directive.verb,
                    &directive.target,
                    &directive.payload,
                    &config.budget,
                );

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
                };

                let sub_result = invoke::invoke(invocation_config)?;
                let resolved = resolve(&sub_result, target_name, config)?;
                sub_responses.push((directive.target.clone(), resolved));
            }

            let refs: Vec<(&str, &str)> = sub_responses
                .iter()
                .map(|(n, t)| (n.as_str(), t.as_str()))
                .collect();
            let aggregated = aggregate_responses(&refs);

            if !config.budget.try_consume() {
                return Err(OrchestrationError::BudgetExhausted);
            }

            let final_result = continue_session(&result.slug, target_name, &aggregated, config)?;
            Ok(final_result.response_text.unwrap_or_default())
        }
        ValidatedDirectiveSet::Handovers(ref handover_directives) => {
            // Target emitted handovers — execute them, then return target's original response
            // (handovers don't produce responses back to the target)
            super::execute_handovers(target_name, handover_directives, config)?;
            Ok(response_text)
        }
    }
}
