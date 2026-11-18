use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    pub fn flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_token_is_not_cancelled() {
        assert!(!CancelToken::default().is_cancelled());
    }

    #[test]
    fn cancel_marks_the_token_cancelled() {
        let token = CancelToken::default();
        token.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn a_clone_shares_the_cancel() {
        let token = CancelToken::default();
        let clone = token.clone();
        clone.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn cancelling_one_turn_leaves_the_next_turn_running() {
        let first = CancelToken::default();
        let next = CancelToken::default();
        first.cancel();
        assert!(!next.is_cancelled());
    }

    #[test]
    fn the_signal_flag_cancels_the_token() {
        let token = CancelToken::default();
        token
            .flag()
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(token.is_cancelled());
    }
}
