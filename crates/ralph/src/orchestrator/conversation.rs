//! Conversation loop for back-and-forth exchanges between two personas.
//!
//! When a target persona responds to an ask directive by issuing a directive
//! back to the originator, the orchestrator enters a conversation loop. Each
//! round continues the responder's session with the latest message, scans for
//! directives, and swaps roles if the responder asks back. The loop terminates
//! when one side finishes without a directive targeting the other, or when the
//! invocation budget is exhausted.

use ralph_core::directive::ValidatedDirectiveSet;

use super::{continue_session, scan_for_directives, OrchestrationConfig, OrchestrationError};

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

        let result = continue_session(responder_session, responder_persona, &message, config)?;
        let response_text = result.response_text.clone().unwrap_or_default();

        // Check if the responder emitted a directive back to the other persona.
        let asks_other = scan_for_directives(&result).and_then(|directives| match directives {
            ValidatedDirectiveSet::Asks(ref asks) => {
                asks.iter().find(|d| d.target == other_persona).cloned()
            }
            ValidatedDirectiveSet::Handovers(_) => None,
        });

        match asks_other {
            Some(directive) => {
                // Swap roles: the other persona becomes the responder.
                message = directive.payload;
                std::mem::swap(&mut responder_session, &mut other_session);
                std::mem::swap(&mut responder_persona, &mut other_persona);
            }
            None => {
                // No directive back — conversation is over.
                return Ok(response_text);
            }
        }
    }
}
