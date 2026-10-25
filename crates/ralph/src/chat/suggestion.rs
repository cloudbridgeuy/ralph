use ralph_core::chat_suggestion::untouched;
use reedline::{Color, ExampleHighlighter, Highlighter, StyledText};
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

    fn latch_untouched(&self, buffer: &str) -> bool {
        let mut slot = self.guard();
        let keep = slot
            .as_deref()
            .is_some_and(|shown| untouched(shown, buffer));
        if !keep {
            *slot = None;
        }
        keep
    }
}

pub(crate) struct SuggestionHighlighter {
    shown: SuggestionSlot,
    color: Color,
    plain: ExampleHighlighter,
}

impl SuggestionHighlighter {
    pub(crate) fn new(shown: SuggestionSlot, color: Color) -> Self {
        Self {
            shown,
            color,
            plain: ExampleHighlighter::default(),
        }
    }
}

impl Highlighter for SuggestionHighlighter {
    fn highlight(&self, line: &str, cursor: usize) -> StyledText {
        if self.shown.latch_untouched(line) {
            let mut styled = StyledText::new();
            styled.push((self.color.normal(), line.to_string()));
            styled
        } else {
            self.plain.highlight(line, cursor)
        }
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

    fn highlighter(text: &str) -> (SuggestionHighlighter, SuggestionSlot) {
        let shown = SuggestionSlot::default();
        shown.set(text.to_string());
        (
            SuggestionHighlighter::new(shown.clone(), Color::Cyan),
            shown,
        )
    }

    #[test]
    fn an_untouched_suggestion_is_one_segment_in_the_suggestion_color() {
        let (highlighter, shown) = highlighter("yes, go");
        let styled = highlighter.highlight("yes, go", 7);
        assert_eq!(styled.buffer.len(), 1);
        assert_eq!(styled.buffer[0].0, Color::Cyan.normal());
        assert_eq!(styled.buffer[0].1, "yes, go");
        assert_eq!(shown.take(), Some("yes, go".to_string()));
    }

    #[test]
    fn other_text_uses_the_plain_highlighter() {
        let (highlighter, _) = highlighter("yes, go");
        let expected = ExampleHighlighter::default().highlight("mine", 4);
        assert_eq!(highlighter.highlight("mine", 4).buffer, expected.buffer);
    }

    #[test]
    fn the_first_edit_latches_to_plain_even_when_the_text_comes_back() {
        let (highlighter, shown) = highlighter("yes, go");
        let expected = ExampleHighlighter::default().highlight("yes, go", 7);
        highlighter.highlight("yes, g", 6);
        assert_eq!(shown.take(), None);
        assert_eq!(highlighter.highlight("yes, go", 7).buffer, expected.buffer);
    }

    #[test]
    fn an_empty_slot_is_plain() {
        let highlighter = SuggestionHighlighter::new(SuggestionSlot::default(), Color::Cyan);
        let expected = ExampleHighlighter::default().highlight("x", 1);
        assert_eq!(highlighter.highlight("x", 1).buffer, expected.buffer);
    }
}
