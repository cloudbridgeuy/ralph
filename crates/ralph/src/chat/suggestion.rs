use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

#[derive(Clone, Default)]
pub(crate) struct SuggestionSlot(Arc<Mutex<Option<String>>>);

impl SuggestionSlot {
    fn guard(&self) -> MutexGuard<'_, Option<String>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn set(&self, text: String) {
        *self.guard() = Some(text);
    }

    pub(crate) fn clear(&self) {
        *self.guard() = None;
    }

    pub(crate) fn take(&self) -> Option<String> {
        self.guard().take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_later_set_replaces_the_earlier_one() {
        let slot = SuggestionSlot::default();
        slot.set("one".to_string());
        slot.set("two".to_string());
        assert_eq!(slot.take(), Some("two".to_string()));
    }

    #[test]
    fn take_empties_the_slot() {
        let slot = SuggestionSlot::default();
        slot.set("one".to_string());
        assert_eq!(slot.take(), Some("one".to_string()));
        assert_eq!(slot.take(), None);
    }

    #[test]
    fn clear_drops_a_pending_suggestion() {
        let slot = SuggestionSlot::default();
        slot.set("one".to_string());
        slot.clear();
        assert_eq!(slot.take(), None);
    }

    #[test]
    fn clones_share_one_slot() {
        let slot = SuggestionSlot::default();
        slot.clone().set("one".to_string());
        assert_eq!(slot.take(), Some("one".to_string()));
    }

    #[test]
    fn a_poisoned_lock_still_works() {
        let slot = SuggestionSlot::default();
        let other = slot.clone();
        let _ = std::thread::spawn(move || {
            let _held = other.0.lock().unwrap();
            panic!("poison");
        })
        .join();
        slot.set("one".to_string());
        assert_eq!(slot.take(), Some("one".to_string()));
    }
}
