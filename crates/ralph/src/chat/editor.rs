use crate::chat::memory::Store;
use crate::clipboard::PASTE_IMAGE;
use ralph_core::chat_draft::{draft_below_separator, editor_prefill};
use ralph_core::chat_memory::Role;
use reedline::{
    default_emacs_keybindings, EditCommand, EditMode, Emacs, FileBackedHistory, KeyCode,
    KeyModifiers, PromptEditMode, Reedline, ReedlineEvent, ReedlineRawEvent,
};
use std::io;
use std::path::Path;
use std::process::Command;

pub(super) const OPEN_EDITOR: &str = "open-editor";
const HISTORY_FILE: &str = "history.txt";
const EDITOR_FILE: &str = "editor.md";
const HISTORY_CAPACITY: usize = 1000;

pub(super) struct ChatEditMode {
    emacs: Emacs,
    esc_pending: bool,
}

pub(super) fn esc_esc(esc_pending: bool, event: ReedlineEvent) -> (bool, ReedlineEvent) {
    match (esc_pending, event) {
        (true, ReedlineEvent::Esc) => (false, ReedlineEvent::Edit(vec![EditCommand::Clear])),
        (false, ReedlineEvent::Esc) => (true, ReedlineEvent::Esc),
        (_, event) => (false, event),
    }
}

impl EditMode for ChatEditMode {
    fn parse_event(&mut self, event: ReedlineRawEvent) -> ReedlineEvent {
        let (esc_pending, event) = esc_esc(self.esc_pending, self.emacs.parse_event(event));
        self.esc_pending = esc_pending;
        event
    }

    fn edit_mode(&self) -> PromptEditMode {
        self.emacs.edit_mode()
    }
}

pub(super) fn build_editor(store_dir: &Path) -> Result<Reedline, Box<dyn std::error::Error>> {
    let history = FileBackedHistory::with_file(HISTORY_CAPACITY, store_dir.join(HISTORY_FILE))?;
    let mut keybindings = default_emacs_keybindings();
    for (modifier, key_code) in [
        (KeyModifiers::ALT, KeyCode::Enter),
        (KeyModifiers::CONTROL, KeyCode::Char('j')),
    ] {
        keybindings.add_binding(
            modifier,
            key_code,
            ReedlineEvent::Edit(vec![EditCommand::InsertNewline]),
        );
    }
    keybindings.add_binding(
        KeyModifiers::CONTROL,
        KeyCode::Char('v'),
        ReedlineEvent::ExecuteHostCommand(PASTE_IMAGE.to_string()),
    );
    keybindings.add_binding(
        KeyModifiers::CONTROL,
        KeyCode::Char('g'),
        ReedlineEvent::ExecuteHostCommand(OPEN_EDITOR.to_string()),
    );
    keybindings.add_binding(
        KeyModifiers::CONTROL,
        KeyCode::Char('o'),
        ReedlineEvent::Edit(vec![EditCommand::Redo]),
    );
    Ok(Reedline::create()
        .with_history(Box::new(history))
        .with_edit_mode(Box::new(ChatEditMode {
            emacs: Emacs::new(keybindings),
            esc_pending: false,
        }))
        .use_bracketed_paste(true))
}

pub(super) fn edit_draft(editor: &mut Reedline, store_dir: &Path) -> io::Result<()> {
    let (entries, _) = Store::new(store_dir.to_path_buf()).load()?;
    let reply = entries
        .iter()
        .rev()
        .find(|entry| entry.role == Role::Assistant)
        .map_or("", |entry| entry.text.as_str());
    let path = store_dir.join(EDITOR_FILE);
    std::fs::write(
        &path,
        editor_prefill(reply, editor.current_buffer_contents()),
    )?;
    let program = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());
    let status = Command::new(program).arg(&path).status()?;
    if !status.success() {
        return Err(io::Error::other(format!("editor exited with {status}")));
    }
    let draft = draft_below_separator(&std::fs::read_to_string(&path)?);
    if !draft.is_empty() {
        editor.run_edit_commands(&[EditCommand::Clear, EditCommand::InsertString(draft)]);
    }
    Ok(())
}
