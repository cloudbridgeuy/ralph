//! Conversation loop for back-and-forth exchanges between two personas.
//!
//! When a target persona responds to an ask directive by issuing a directive
//! back to the originator, the orchestrator enters a conversation loop. Each
//! round continues the responder's session with the latest message, scans for
//! directives, and swaps roles if the responder asks back. The loop terminates
//! when one side finishes without a directive targeting the other, or when the
//! invocation budget is exhausted.

use ralph_core::directive::{aggregate_responses, Directive, DirectiveVerb, ValidatedDirectiveSet};

use super::{
    ask, continue_session, display, scan_for_directives, OrchestrationConfig, OrchestrationError,
};

/// Categorized outcome of scanning a conversation responder's output.
///
/// Pure data type — the conversation loop matches on this to decide
/// its next action without re-inspecting raw directives.
#[derive(Debug)]
enum ConversationAction {
    /// The responder emitted an ask targeting the other conversant.
    /// Swap roles and continue the conversation.
    AskOther(Directive),
    /// The responder emitted ask directives to third-party personas
    /// (not the other conversant). Resolve them, feed results back
    /// to the responder, then re-scan.
    ThirdParty(Vec<Directive>),
    /// No directives, or only handovers. Conversation is over.
    Done,
}

/// Categorize directives from a conversation responder's output.
///
/// Pure function — examines the directive set and determines the
/// conversation loop's next action based on whether directives
/// target the other conversant, third parties, or nobody.
///
/// Rules:
/// - Ask targeting `other_persona` → `AskOther` (swap roles)
/// - Asks NOT targeting `other_persona` → `ThirdParty` (resolve sub-directives)
/// - Handovers → `Done` (conversation-ending; handovers are fire-and-forget)
/// - No directives → `Done`
fn categorize_conversation_directives(
    directives: Option<ValidatedDirectiveSet>,
    other_persona: &str,
) -> ConversationAction {
    let directives = match directives {
        Some(d) => d,
        None => return ConversationAction::Done,
    };

    match directives {
        ValidatedDirectiveSet::Handovers(_) => ConversationAction::Done,
        ValidatedDirectiveSet::Asks(asks) => {
            match asks.iter().find(|d| d.target == other_persona).cloned() {
                Some(directive) => ConversationAction::AskOther(directive),
                None => ConversationAction::ThirdParty(asks),
            }
        }
    }
}

/// Configuration for a two-persona conversation loop.
///
/// Groups the session and persona identifiers for both sides of the
/// conversation plus the initial message, keeping [`conversation_loop`]
/// within the max-5-argument limit.
pub struct ConversationConfig<'a> {
    /// Session slug for persona A (the originator).
    pub a_session: &'a str,
    /// Persona name for A.
    pub a_persona: &'a str,
    /// Session slug for persona B (the target).
    pub b_session: &'a str,
    /// Persona name for B.
    pub b_persona: &'a str,
    /// The message that kicks off the conversation loop.
    ///
    /// This is the payload from B's directive targeting A. The first action in
    /// the loop is to continue A's session with this message.
    pub initial_message: &'a str,
}

/// Run a back-and-forth conversation loop between two personas.
///
/// The loop alternates which persona responds. On each iteration:
/// 1. Continue the current responder's session with the latest message.
/// 2. Scan the result for directives.
/// 3. If a directive targets the other persona, extract the payload and swap
///    roles for the next iteration.
/// 4. If no directive targets the other persona, return the final response text.
///
/// The loop also terminates immediately when the invocation budget is exhausted.
pub fn conversation_loop(
    conv: &ConversationConfig,
    config: &OrchestrationConfig,
) -> Result<String, OrchestrationError> {
    // Current responder starts as A (the originator), because B already
    // responded and its directive payload is the initial_message for A.
    let mut responder_session = conv.a_session;
    let mut responder_persona = conv.a_persona;
    let mut other_persona = conv.b_persona;
    let mut other_session = conv.b_session;
    let mut message = conv.initial_message.to_string();

    loop {
        if !config.budget.try_consume() {
            return Err(OrchestrationError::BudgetExhausted);
        }

        display::print_routing_status(
            other_persona,
            &DirectiveVerb::Ask,
            responder_persona,
            &message,
            &config.budget,
        );

        let result = continue_session(
            responder_session,
            responder_persona,
            &message,
            Some(other_persona),
            config,
        )?;
        let response_text = result.response_text.clone().unwrap_or_default();

        let directives = scan_for_directives(&result);
        let action = categorize_conversation_directives(directives, other_persona);

        match action {
            ConversationAction::AskOther(directive) => {
                // Swap roles: the other persona becomes the responder.
                message = directive.payload;
                std::mem::swap(&mut responder_session, &mut other_session);
                std::mem::swap(&mut responder_persona, &mut other_persona);
            }
            ConversationAction::ThirdParty(third_party_asks) => {
                // Resolve third-party asks: invoke targets, aggregate responses,
                // continue the current responder with the results, then re-scan.
                let sub_responses = ask::resolve_parallel_asks(
                    &third_party_asks,
                    responder_persona,
                    responder_session,
                    config,
                )?;
                let refs: Vec<(&str, &str)> = sub_responses
                    .iter()
                    .map(|(n, t)| (n.as_str(), t.as_str()))
                    .collect();
                message = aggregate_responses(&refs);
                // Don't swap roles — continue the same responder with third-party results.
            }
            ConversationAction::Done => {
                return Ok(response_text);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::directive::DirectiveVerb;

    fn ask_directive(target: &str, payload: &str) -> Directive {
        Directive {
            verb: DirectiveVerb::Ask,
            target: target.to_string(),
            payload: payload.to_string(),
        }
    }

    fn handover_directive(target: &str, payload: &str) -> Directive {
        Directive {
            verb: DirectiveVerb::Handover,
            target: target.to_string(),
            payload: payload.to_string(),
        }
    }

    #[test]
    fn categorize_none_returns_done() {
        let action = categorize_conversation_directives(None, "alice");
        assert!(matches!(action, ConversationAction::Done));
    }

    #[test]
    fn categorize_handovers_returns_done() {
        let directives = Some(ValidatedDirectiveSet::Handovers(vec![handover_directive(
            "deployer",
            "deploy it",
        )]));
        let action = categorize_conversation_directives(directives, "alice");
        assert!(matches!(action, ConversationAction::Done));
    }

    #[test]
    fn categorize_single_ask_targeting_other_returns_ask_other() {
        let directives = Some(ValidatedDirectiveSet::Asks(vec![ask_directive(
            "alice",
            "what do you think?",
        )]));
        let action = categorize_conversation_directives(directives, "alice");
        match action {
            ConversationAction::AskOther(d) => {
                assert_eq!(d.target, "alice");
                assert_eq!(d.payload, "what do you think?");
            }
            other => panic!("expected AskOther, got {other:?}"),
        }
    }

    #[test]
    fn categorize_single_ask_targeting_third_party_returns_third_party() {
        let directives = Some(ValidatedDirectiveSet::Asks(vec![ask_directive(
            "charlie", "help me",
        )]));
        let action = categorize_conversation_directives(directives, "alice");
        match action {
            ConversationAction::ThirdParty(asks) => {
                assert_eq!(asks.len(), 1);
                assert_eq!(asks[0].target, "charlie");
            }
            other => panic!("expected ThirdParty, got {other:?}"),
        }
    }

    #[test]
    fn categorize_multiple_asks_one_targeting_other_returns_ask_other() {
        let directives = Some(ValidatedDirectiveSet::Asks(vec![
            ask_directive("charlie", "help me"),
            ask_directive("alice", "what do you think?"),
            ask_directive("dave", "check this"),
        ]));
        let action = categorize_conversation_directives(directives, "alice");
        match action {
            ConversationAction::AskOther(d) => {
                assert_eq!(d.target, "alice");
                assert_eq!(d.payload, "what do you think?");
            }
            other => panic!("expected AskOther, got {other:?}"),
        }
    }

    #[test]
    fn categorize_multiple_asks_none_targeting_other_returns_third_party() {
        let directives = Some(ValidatedDirectiveSet::Asks(vec![
            ask_directive("charlie", "help me"),
            ask_directive("dave", "check this"),
        ]));
        let action = categorize_conversation_directives(directives, "alice");
        match action {
            ConversationAction::ThirdParty(asks) => {
                assert_eq!(asks.len(), 2);
                assert_eq!(asks[0].target, "charlie");
                assert_eq!(asks[1].target, "dave");
            }
            other => panic!("expected ThirdParty, got {other:?}"),
        }
    }

    #[test]
    fn categorize_mixed_asks_with_other_extracts_first_match() {
        let directives = Some(ValidatedDirectiveSet::Asks(vec![
            ask_directive("charlie", "third party question"),
            ask_directive("alice", "first question to other"),
            ask_directive("alice", "second question to other"),
        ]));
        let action = categorize_conversation_directives(directives, "alice");
        match action {
            ConversationAction::AskOther(d) => {
                assert_eq!(d.target, "alice");
                assert_eq!(d.payload, "first question to other");
            }
            other => panic!("expected AskOther, got {other:?}"),
        }
    }
}
