use ralph_core::chat_suggestion::untouched;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

#[derive(Clone)]
pub struct SuggestionCell<T = String>(Arc<Mutex<Option<T>>>);

impl<T> Default for SuggestionCell<T> {
    fn default() -> Self {
        Self(Arc::default())
    }
}

impl<T> SuggestionCell<T> {
    fn guard(&self) -> MutexGuard<'_, Option<T>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn set(&self, suggestion: T) {
        *self.guard() = Some(suggestion);
    }

    pub fn clear(&self) {
        *self.guard() = None;
    }

    pub fn take(&self) -> Option<T> {
        self.guard().take()
    }
}

impl SuggestionCell {
    pub fn latch_untouched(&self, buffer: &str) -> bool {
        let mut slot = self.guard();
        let keep = slot.as_ref().is_some_and(|shown| untouched(shown, buffer));
        if !keep {
            *slot = None;
        }
        keep
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> String {
        text.to_string()
    }

    #[test]
    fn a_later_set_replaces_the_earlier_one() {
        let slot = SuggestionCell::default();
        slot.set(plain("one"));
        slot.set(plain("two"));
        assert_eq!(slot.take(), Some(plain("two")));
    }

    #[test]
    fn take_empties_the_slot() {
        let slot = SuggestionCell::default();
        slot.set(plain("one"));
        assert_eq!(slot.take(), Some(plain("one")));
        assert_eq!(slot.take(), None);
    }

    #[test]
    fn clear_drops_a_pending_suggestion() {
        let slot = SuggestionCell::default();
        slot.set(plain("one"));
        slot.clear();
        assert_eq!(slot.take(), None);
    }

    #[test]
    fn clones_share_one_slot() {
        let slot = SuggestionCell::default();
        slot.clone().set(plain("one"));
        assert_eq!(slot.take(), Some(plain("one")));
    }

    #[test]
    fn a_poisoned_lock_still_works() {
        let slot = SuggestionCell::default();
        let other = slot.clone();
        let _ = std::thread::spawn(move || {
            let _held = other.0.lock().unwrap();
            panic!("poison");
        })
        .join();
        slot.set(plain("one"));
        assert_eq!(slot.take(), Some(plain("one")));
    }
}
