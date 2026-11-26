use crate::chat::editor::history;
use crate::chat::queue::{exit_notice, unsent_texts};
use crate::chat::session::save_unsent;
use crossterm::queue;
use crossterm::style::Print;
use crossterm::terminal::{Clear, ClearType};
use ralph_core::chat_stash::Stash;
use ralph_server::warn::{warn, warn_if_err};
use reedline::{EditCommand, History, Reedline};
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

fn write_hard_exit_notice(out: &mut impl Write, notice: &str) -> io::Result<()> {
    queue!(
        out,
        Print("\r\n"),
        Clear(ClearType::FromCursorDown),
        Print(notice),
        Print("\r\n")
    )?;
    out.flush()
}

#[derive(Debug, Clone, Default)]
pub(super) struct DraftStash(Arc<Mutex<Stash>>);

impl DraftStash {
    fn guard(&self) -> MutexGuard<'_, Stash> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn push(&self, editor: &mut Reedline) {
        if editor.current_buffer_contents().is_empty() {
            return;
        }
        self.guard().push(editor.current_buffer_contents());
        editor.run_edit_commands(&[EditCommand::Clear]);
        editor.reset_undo_stack();
    }

    pub(super) fn restore(&self, editor: &mut Reedline) {
        if let Some(draft) = self.guard().restore(editor.current_buffer_contents()) {
            editor.run_edit_commands(&[EditCommand::Clear, EditCommand::InsertString(draft)]);
        }
    }

    pub(super) fn displace<T>(&self, suggestion: Option<T>) -> Option<T> {
        self.guard().displace(suggestion)
    }

    pub(super) fn save(&self, history: &mut dyn History) -> Option<String> {
        let mut stash = self.guard();
        let lost = unsent_texts(&[], &stash.restore("")?);
        let saved = save_unsent(history, &lost);
        let synced = history.sync();
        let notice = exit_notice(lost.len(), saved && synced.is_ok());
        warn_if_err(synced, "Failed to save chat history");
        notice
    }

    pub(super) fn save_before_hard_exit(&self, store_dir: &Path) {
        match history(store_dir) {
            Ok(mut history) => {
                if let Some(notice) = self.save(&mut history) {
                    let _ = write_hard_exit_notice(&mut io::stderr(), &notice);
                }
            }
            Err(error) => warn(format!("Failed to save chat history: {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reedline::EditCommand;

    fn editor_with(draft: &str) -> Reedline {
        let mut editor = Reedline::create();
        editor.run_edit_commands(&[EditCommand::InsertString(draft.to_string())]);
        editor
    }

    #[test]
    fn push_clears_the_prompt_and_restore_fills_it_with_the_cursor_at_the_end() {
        let stash = DraftStash::default();
        let mut editor = editor_with("half a thought");
        stash.push(&mut editor);
        assert_eq!(editor.current_buffer_contents(), "");
        stash.restore(&mut editor);
        assert_eq!(editor.current_buffer_contents(), "half a thought");
        assert_eq!(editor.current_insertion_point(), "half a thought".len());
    }

    #[test]
    fn restore_puts_the_stash_before_the_prompt_text() {
        let stash = DraftStash::default();
        let mut editor = editor_with("stash");
        stash.push(&mut editor);
        editor.run_edit_commands(&[EditCommand::InsertString("held".to_string())]);
        stash.restore(&mut editor);
        assert_eq!(editor.current_buffer_contents(), "stash\n\nheld");
    }

    #[test]
    fn an_empty_stash_leaves_the_prompt_alone() {
        let mut editor = editor_with("held");
        DraftStash::default().restore(&mut editor);
        assert_eq!(editor.current_buffer_contents(), "held");
    }

    #[test]
    fn undo_and_redo_after_a_push_do_not_bring_the_draft_back() {
        let stash = DraftStash::default();
        let mut editor = editor_with("half ");
        editor.run_edit_commands(&[EditCommand::InsertString("a thought".to_string())]);
        editor.run_edit_commands(&[EditCommand::Undo]);
        stash.push(&mut editor);
        for command in [EditCommand::Undo, EditCommand::Undo, EditCommand::Redo] {
            editor.run_edit_commands(std::slice::from_ref(&command));
            assert_eq!(editor.current_buffer_contents(), "", "{command:?}");
        }
        stash.restore(&mut editor);
        assert_eq!(editor.current_buffer_contents(), "half ");
    }

    #[test]
    fn a_draft_with_many_undo_points_does_not_come_back() {
        let mut editor = Reedline::create();
        for _ in 0..10_050 {
            editor.run_edit_commands(&[EditCommand::InsertString("a".to_string())]);
        }
        DraftStash::default().push(&mut editor);
        editor.run_edit_commands(&[EditCommand::Undo]);
        assert_eq!(editor.current_buffer_contents().len(), 0);
    }

    #[test]
    fn a_hard_exit_notice_clears_the_rows_below_the_cursor_first() {
        let mut out = Vec::new();
        write_hard_exit_notice(&mut out, "saved").unwrap();
        let mut expected = b"\r\n".to_vec();
        queue!(expected, Clear(ClearType::FromCursorDown)).unwrap();
        expected.extend_from_slice(b"saved\r\n");
        assert_eq!(out, expected);
    }

    #[test]
    fn a_push_on_an_empty_prompt_keeps_the_undo_history() {
        let mut editor = editor_with("cleared");
        editor.run_edit_commands(&[EditCommand::Clear]);
        DraftStash::default().push(&mut editor);
        editor.run_edit_commands(&[EditCommand::Undo]);
        assert_eq!(editor.current_buffer_contents(), "cleared");
    }

    #[test]
    fn a_hard_exit_saves_the_stash_to_the_chat_history() {
        let dir = tempfile::tempdir().unwrap();
        let stash = DraftStash::default();
        stash.push(&mut editor_with("unsent stash"));
        stash.save_before_hard_exit(dir.path());
        let history = std::fs::read_to_string(dir.path().join("history.txt")).unwrap_or_default();
        assert!(history.contains("unsent stash"), "{history:?}");
    }

    #[test]
    fn a_normal_exit_saves_the_stash_once_with_the_unsent_notice() {
        let dir = tempfile::tempdir().unwrap();
        let mut history = crate::chat::editor::history(dir.path()).unwrap();
        let stash = DraftStash::default();
        stash.push(&mut editor_with("unsent stash"));
        assert_eq!(
            stash.save(&mut history).as_deref(),
            Some("1 queued message not sent; saved to history.")
        );
        assert_eq!(stash.save(&mut history), None);
    }
}
