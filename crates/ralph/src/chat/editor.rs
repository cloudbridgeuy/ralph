use crate::clipboard::PASTE_IMAGE;
use reedline::{
    default_emacs_keybindings, EditCommand, EditMode, Emacs, FileBackedHistory, KeyCode,
    KeyModifiers, PromptEditMode, Reedline, ReedlineEvent, ReedlineRawEvent,
};
use std::path::Path;
use std::process::Command;

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
        ReedlineEvent::OpenEditor,
    );
    keybindings.add_binding(
        KeyModifiers::CONTROL,
        KeyCode::Char('o'),
        ReedlineEvent::Edit(vec![EditCommand::Redo]),
    );
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());
    Ok(Reedline::create()
        .with_history(Box::new(history))
        .with_edit_mode(Box::new(ChatEditMode {
            emacs: Emacs::new(keybindings),
            esc_pending: false,
        }))
        .with_buffer_editor(Command::new(editor), store_dir.join(EDITOR_FILE))
        .use_bracketed_paste(true))
}
