//! Edit mode of the chat prompt (Functional Core).

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EditMode {
    #[default]
    Emacs,
    Vi,
}

/// The mode the editor reports at render time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptMode {
    Emacs,
    ViInsert,
    ViNormal,
}

/// Reads `[chat] edit_mode`; an unknown value falls back to emacs with a warning.
pub fn resolve_edit_mode(config: Option<&str>) -> (EditMode, Option<String>) {
    match config.map(str::trim) {
        None | Some("emacs") => (EditMode::Emacs, None),
        Some("vi") => (EditMode::Vi, None),
        Some(other) => (
            EditMode::Emacs,
            Some(format!(
                "[chat] edit_mode = {other:?} is not \"emacs\" or \"vi\"; using \"emacs\""
            )),
        ),
    }
}

/// Esc Esc clears the draft only in emacs mode; in vi mode Esc leaves insert mode.
pub fn esc_clears_draft(mode: EditMode) -> bool {
    mode == EditMode::Emacs
}

pub fn prompt_indicator(mode: PromptMode) -> &'static str {
    match mode {
        PromptMode::Emacs | PromptMode::ViInsert => ">>> ",
        PromptMode::ViNormal => "::: ",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_mode_defaults_to_emacs() {
        assert_eq!(resolve_edit_mode(None), (EditMode::Emacs, None));
        assert_eq!(resolve_edit_mode(Some("emacs")), (EditMode::Emacs, None));
    }

    #[test]
    fn edit_mode_reads_vi() {
        assert_eq!(resolve_edit_mode(Some(" vi ")), (EditMode::Vi, None));
    }

    #[test]
    fn unknown_edit_mode_warns_and_uses_emacs() {
        let (mode, warning) = resolve_edit_mode(Some("helix"));
        assert_eq!(mode, EditMode::Emacs);
        assert!(warning.is_some_and(|w| w.contains("\"helix\"")));
    }

    #[test]
    fn only_emacs_clears_on_esc_esc() {
        assert!(esc_clears_draft(EditMode::Emacs));
        assert!(!esc_clears_draft(EditMode::Vi));
    }

    #[test]
    fn indicator_shows_vi_mode() {
        assert_eq!(prompt_indicator(PromptMode::Emacs), ">>> ");
        assert_eq!(prompt_indicator(PromptMode::ViInsert), ">>> ");
        assert_eq!(prompt_indicator(PromptMode::ViNormal), "::: ");
    }
}
