use crate::chat_thread::{AutoMode, ThreadMeta};

pub fn with_auto(meta: &ThreadMeta, auto: Option<AutoMode>) -> Option<ThreadMeta> {
    auto.filter(|auto| meta.auto != Some(*auto))
        .map(|auto| ThreadMeta {
            auto: Some(auto),
            ..meta.clone()
        })
}

pub fn clear_closed(meta: &ThreadMeta) -> Option<ThreadMeta> {
    meta.closed.then(|| ThreadMeta {
        closed: false,
        ..meta.clone()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta_with(auto: Option<AutoMode>, closed: bool) -> ThreadMeta {
        ThreadMeta {
            base: 1,
            created: "t".to_string(),
            merged: None,
            parked: None,
            auto,
            closed,
        }
    }

    #[test]
    fn with_auto_sets_or_replaces_the_choice_and_ignores_none() {
        let fresh = meta_with(None, false);
        assert_eq!(
            with_auto(&fresh, Some(AutoMode::Merge)),
            Some(meta_with(Some(AutoMode::Merge), false))
        );
        let stored = meta_with(Some(AutoMode::Merge), false);
        assert_eq!(
            with_auto(&stored, Some(AutoMode::Drop)),
            Some(meta_with(Some(AutoMode::Drop), false))
        );
        assert_eq!(with_auto(&stored, Some(AutoMode::Merge)), None);
        assert_eq!(with_auto(&stored, None), None);
    }

    #[test]
    fn clear_closed_keeps_auto_and_skips_an_open_thread() {
        assert_eq!(
            clear_closed(&meta_with(Some(AutoMode::Drop), true)),
            Some(meta_with(Some(AutoMode::Drop), false))
        );
        assert_eq!(clear_closed(&meta_with(None, false)), None);
    }
}
