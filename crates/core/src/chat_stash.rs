const JOIN: &str = "\n\n";

#[derive(Debug, Default)]
pub struct Stash(Option<String>);

impl Stash {
    pub fn push(&mut self, draft: &str) {
        self.0 = match self.0.take() {
            Some(stashed) => Some(joined(stashed, draft)),
            None => (!draft.is_empty()).then(|| draft.to_string()),
        };
    }

    pub fn restore(&mut self, held: &str) -> Option<String> {
        self.0.take().map(|stashed| joined(stashed, held))
    }

    pub fn displace<T>(&self, suggestion: Option<T>) -> Option<T> {
        suggestion.filter(|_| self.0.is_none())
    }
}

fn joined(first: String, then: &str) -> String {
    if then.is_empty() {
        first
    } else {
        format!("{first}{JOIN}{then}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_status::Status;
    use crate::chat_suggestion::{turn_end, Next, Suggestion, TurnEnd};

    fn stashed(drafts: &[&str]) -> Stash {
        let mut stash = Stash::default();
        for draft in drafts {
            stash.push(draft);
        }
        stash
    }

    #[test]
    fn the_stash_comes_back_once() {
        let mut stash = stashed(&["half a thought"]);
        assert_eq!(stash.restore(""), Some("half a thought".to_string()));
        assert_eq!(stash.restore(""), None);
    }

    #[test]
    fn a_second_push_appends_after_a_blank_line() {
        assert_eq!(
            stashed(&["one", "two"]).restore(""),
            Some("one\n\ntwo".to_string())
        );
    }

    #[test]
    fn an_empty_push_changes_nothing() {
        assert_eq!(stashed(&[""]).restore("held"), None);
        assert_eq!(stashed(&["one", ""]).restore(""), Some("one".to_string()));
    }

    #[test]
    fn the_stash_goes_before_the_held_text() {
        assert_eq!(
            stashed(&["stash"]).restore("restored\n\nqueued"),
            Some("stash\n\nrestored\n\nqueued".to_string())
        );
    }

    #[test]
    fn a_stash_displaces_a_suggestion() {
        assert_eq!(stashed(&["stash"]).displace(Some("next")), None);
    }

    #[test]
    fn an_empty_stash_keeps_the_suggestion() {
        assert_eq!(Stash::default().displace(Some("next")), Some("next"));
    }

    #[test]
    fn a_stash_stops_a_turn_end_auto_hold_and_prefill() {
        let suggestion = Some(Suggestion::Open("/hold".to_string()));
        let next = turn_end(TurnEnd {
            auto_hold: true,
            replied: true,
            draft: None,
            suggestion: stashed(&["stash"]).displace(suggestion),
            status: &Status::default(),
        });
        assert_eq!(next, Next::Nothing);
    }
}
