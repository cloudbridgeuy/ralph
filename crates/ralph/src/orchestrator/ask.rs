//! Ask executor for multi-agent directive routing.
//!
//! Implements the ask round-trip: originator emits ask directives, targets are
//! invoked in parallel, responses are aggregated, and the originator is
//! continued with the aggregated prompt. Sub-directives from targets are
//! resolved sequentially after all parallel invocations complete.

use ralph_core::directive::{aggregate_responses, Directive, ValidatedDirectiveSet};

use super::conversation::{self, ConversationConfig};
use super::{
    continue_session, display, parallel, scan_for_directives, OrchestrationConfig,
    OrchestrationError,
};
use crate::invoke::InvocationResult;

/// Invoke ask directives in parallel, resolve sub-directives, and aggregate responses.
///
/// Returns a list of `(target_name, resolved_text)` pairs in directive order.
/// This is the shared invoke-resolve-aggregate pattern used by [`execute_asks`],
/// [`resolve`], and the conversation loop's third-party branch.
pub(super) fn resolve_parallel_asks(
    directives: &[Directive],
    originator_persona: &str,
    originator_session_slug: &str,
    config: &OrchestrationConfig,
) -> Result<Vec<(String, String)>, OrchestrationError> {
    let results = parallel::parallel_invoke(directives, originator_persona, config);
    let mut responses: Vec<(String, String)> = Vec::new();

    for (directive, result) in directives.iter().zip(results) {
        let result = result?;
        let resolved = resolve(&result, originator_persona, originator_session_slug, config)?;
        responses.push((directive.target.clone(), resolved));
    }

    Ok(responses)
}

/// Execute ask directives in parallel and continue the originator.
///
/// Invokes all target personas concurrently via [`parallel::parallel_invoke`],
/// then resolves sub-directives sequentially. After all targets have responded,
/// aggregates the responses into a single prompt and continues the originator's
/// session via [`continue_session`]. If the continuation produces further
/// directives, recurses through [`super::orchestrate_inner`].
pub fn execute_asks(
    directives: &[Directive],
    originator: &InvocationResult,
    config: &OrchestrationConfig,
) -> Result<(), OrchestrationError> {
    let originator_name = originator.display_name();

    // Phase 1: invoke all targets in parallel, then resolve sub-directives sequentially
    let responses = resolve_parallel_asks(directives, originator_name, &originator.slug, config)?;

    // Phase 2: aggregate responses and continue the originator
    let response_refs: Vec<(&str, &str)> = responses
        .iter()
        .map(|(name, text)| (name.as_str(), text.as_str()))
        .collect();
    let aggregated = aggregate_responses(&response_refs);

    if !config.budget.try_consume() {
        return Err(OrchestrationError::BudgetExhausted);
    }

    display::print_persona_banner(originator_name);
    let continuation_result =
        continue_session(&originator.slug, originator_name, &aggregated, None, config)?;

    // Phase 3: scan continuation for more directives and recurse
    if let Some(sub_directives) = scan_for_directives(&continuation_result, &config.known_personas)
    {
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
/// 3. **Sub-directive back to originator**: enter a conversation loop where the
///    two personas exchange messages until one side finishes without a directive
///    targeting the other.
pub(super) fn resolve(
    result: &InvocationResult,
    originator_persona: &str,
    originator_session_slug: &str,
    config: &OrchestrationConfig,
) -> Result<String, OrchestrationError> {
    let response_text = result.response_text.clone().unwrap_or_default();

    let sub_directives = match scan_for_directives(result, &config.known_personas) {
        Some(d) => d,
        None => return Ok(response_text),
    };

    // Check if any sub-directive targets the originator (conversation loop)
    let originator_directive = match &sub_directives {
        ValidatedDirectiveSet::Asks(directives) => directives
            .iter()
            .find(|d| d.target == originator_persona)
            .cloned(),
        ValidatedDirectiveSet::Handovers(_) => None,
    };

    let target_name = result.display_name();

    if let Some(directive) = originator_directive {
        // Target is asking the originator back — enter conversation loop.
        let conv_config = ConversationConfig {
            a_session: originator_session_slug,
            a_persona: originator_persona,
            b_session: &result.slug,
            b_persona: target_name,
            initial_message: &directive.payload,
        };
        return conversation::conversation_loop(&conv_config, config);
    }

    // Sub-directives to new personas — execute them
    match sub_directives {
        ValidatedDirectiveSet::Asks(ref ask_directives) => {
            // Target emitted asks: invoke those in parallel, resolve sequentially,
            // aggregate, continue target, return final
            let sub_responses =
                resolve_parallel_asks(ask_directives, target_name, &result.slug, config)?;
            let refs: Vec<(&str, &str)> = sub_responses
                .iter()
                .map(|(n, t)| (n.as_str(), t.as_str()))
                .collect();
            let aggregated = aggregate_responses(&refs);

            if !config.budget.try_consume() {
                return Err(OrchestrationError::BudgetExhausted);
            }

            let final_result = continue_session(
                &result.slug,
                target_name,
                &aggregated,
                Some(originator_persona),
                config,
            )?;
            resolve(
                &final_result,
                originator_persona,
                originator_session_slug,
                config,
            )
        }
        ValidatedDirectiveSet::Handovers(ref handover_directives) => {
            // Target emitted handovers — execute them, then return target's original response
            // (handovers don't produce responses back to the target)
            super::execute_handovers(target_name, handover_directives, config)?;
            Ok(response_text)
        }
    }
}
