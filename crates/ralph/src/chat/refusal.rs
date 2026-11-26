use crate::chat::queue::{refusal_action, refused_draft, Refusal};
use crate::chat::session::{print_exit_notice, save_unsent};
use crate::signal;
use reedline::{EditCommand, Reedline};

pub(super) fn refuse_or_exit(
    editor: &mut Reedline,
    line: &str,
    carried: Vec<String>,
) -> Result<bool, Box<dyn std::error::Error>> {
    let buffer = editor.current_buffer_contents().to_string();
    match refusal_action(signal::is_terminated(), line, &carried, &buffer) {
        Refusal::Save(lost) => {
            let saved = save_unsent(editor.history_mut(), &lost);
            print_exit_notice(lost.len(), saved);
            editor.sync_history()?;
            Ok(true)
        }
        Refusal::Restore => {
            restore_prompt(editor, line, carried);
            Ok(false)
        }
    }
}

pub(super) fn restore_prompt(editor: &mut Reedline, line: &str, carried: Vec<String>) {
    let draft = refused_draft(line, &carried, editor.current_buffer_contents());
    editor.run_edit_commands(&[EditCommand::Clear, EditCommand::InsertString(draft)]);
}
