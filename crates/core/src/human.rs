//! Human-in-the-loop directive classification.
//!
//! Pure functions for classifying directive targets as human vs persona
//! and partitioning directive sets. Following the Functional Core pattern,
//! all functions operate on data provided as arguments — no I/O.

use crate::directive::Directive;

/// Classification of a directive target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectiveTarget {
    /// Target is a named persona (agent).
    Persona(String),
    /// Target is the human operator.
    Human,
}

/// Reserved target name for human-in-the-loop.
pub const HUMAN_TARGET: &str = "human";

/// Classify a directive's target string.
///
/// Returns `Human` for the reserved `"human"` target, otherwise `Persona`.
pub fn classify_target(target: &str) -> DirectiveTarget {
    if target == HUMAN_TARGET {
        DirectiveTarget::Human
    } else {
        DirectiveTarget::Persona(target.to_string())
    }
}

/// Partition directives into human-targeted and persona-targeted.
///
/// Returns `(human_directives, persona_directives)`.
pub fn partition_directives(directives: &[Directive]) -> (Vec<&Directive>, Vec<&Directive>) {
    let mut human = Vec::new();
    let mut persona = Vec::new();

    for d in directives {
        match classify_target(&d.target) {
            DirectiveTarget::Human => human.push(d),
            DirectiveTarget::Persona(_) => persona.push(d),
        }
    }

    (human, persona)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directive::DirectiveVerb;

    #[test]
    fn classify_target_human() {
        assert_eq!(classify_target("human"), DirectiveTarget::Human);
    }

    #[test]
    fn classify_target_persona() {
        assert_eq!(
            classify_target("storyteller"),
            DirectiveTarget::Persona("storyteller".to_string())
        );
    }

    #[test]
    fn classify_target_empty_string() {
        assert_eq!(
            classify_target(""),
            DirectiveTarget::Persona("".to_string())
        );
    }

    #[test]
    fn classify_target_case_sensitive() {
        assert_eq!(
            classify_target("Human"),
            DirectiveTarget::Persona("Human".to_string())
        );
    }

    #[test]
    fn partition_mixed_targets() {
        let directives = vec![
            Directive {
                verb: DirectiveVerb::Ask,
                target: "human".to_string(),
                payload: "What do you think?".to_string(),
            },
            Directive {
                verb: DirectiveVerb::Ask,
                target: "editor-agent".to_string(),
                payload: "Check pacing".to_string(),
            },
            Directive {
                verb: DirectiveVerb::Ask,
                target: "human".to_string(),
                payload: "Another question".to_string(),
            },
        ];
        let (human, persona) = partition_directives(&directives);
        assert_eq!(human.len(), 2);
        assert_eq!(persona.len(), 1);
        assert_eq!(human[0].target, "human");
        assert_eq!(human[1].target, "human");
        assert_eq!(persona[0].target, "editor-agent");
    }

    #[test]
    fn partition_all_human() {
        let directives = vec![Directive {
            verb: DirectiveVerb::Ask,
            target: "human".to_string(),
            payload: "Question".to_string(),
        }];
        let (human, persona) = partition_directives(&directives);
        assert_eq!(human.len(), 1);
        assert!(persona.is_empty());
    }

    #[test]
    fn partition_no_human() {
        let directives = vec![Directive {
            verb: DirectiveVerb::Ask,
            target: "reviewer".to_string(),
            payload: "Review".to_string(),
        }];
        let (human, persona) = partition_directives(&directives);
        assert!(human.is_empty());
        assert_eq!(persona.len(), 1);
    }

    #[test]
    fn partition_empty() {
        let directives: Vec<Directive> = vec![];
        let (human, persona) = partition_directives(&directives);
        assert!(human.is_empty());
        assert!(persona.is_empty());
    }
}
