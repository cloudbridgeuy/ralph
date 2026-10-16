use crate::clipboard::PASTE_IMAGE;
use ralph_core::chat_edit_mode::{esc_clears_draft, EditMode as EditModeConfig};
use reedline::{
    default_emacs_keybindings, default_vi_insert_keybindings, default_vi_normal_keybindings,
    default_vi_visual_keybindings, EditCommand, EditMode, Emacs, FileBackedHistory, KeyCode,
    KeyModifiers, Keybindings, PromptEditMode, PromptViMode, Reedline, ReedlineEvent,
    ReedlineRawEvent, Vi,
};
use std::path::Path;
use std::process::Command;

const HISTORY_FILE: &str = "history.txt";
const EDITOR_FILE: &str = "editor.md";
const HISTORY_CAPACITY: usize = 1000;

pub(super) struct ChatEditMode {
    inner: Box<dyn EditMode>,
    esc_clears: bool,
    esc_pending: bool,
}

pub(super) fn esc_esc(esc_pending: bool, event: ReedlineEvent) -> (bool, ReedlineEvent) {
    match (esc_pending, event) {
        (true, ReedlineEvent::Esc) => (false, ReedlineEvent::Edit(vec![EditCommand::Clear])),
        (false, ReedlineEvent::Esc) => (true, ReedlineEvent::Esc),
        (_, event) => (false, event),
    }
}

/// Vi visual mode is off: a key that entered it is answered with a switch
/// back to normal mode, before reedline reads the mode or paints.
pub(super) fn visual_reset(mode: PromptEditMode) -> Option<ReedlineEvent> {
    matches!(mode, PromptEditMode::Vi(PromptViMode::Visual)).then_some(ReedlineEvent::SwitchMode(
        PromptEditMode::Vi(PromptViMode::Normal),
    ))
}

impl EditMode for ChatEditMode {
    fn parse_event(&mut self, event: ReedlineRawEvent) -> ReedlineEvent {
        let event = self.inner.parse_event(event);
        if let Some(reset) = visual_reset(self.inner.edit_mode()) {
            self.inner.handle_mode_specific_event(reset);
            return ReedlineEvent::None;
        }
        if !self.esc_clears {
            return event;
        }
        let (esc_pending, event) = esc_esc(self.esc_pending, event);
        self.esc_pending = esc_pending;
        event
    }

    fn edit_mode(&self) -> PromptEditMode {
        self.inner.edit_mode()
    }
}

fn add_chat_bindings(keybindings: &mut Keybindings) {
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
}

pub(super) fn emacs_keybindings() -> Keybindings {
    let mut keybindings = default_emacs_keybindings();
    add_chat_bindings(&mut keybindings);
    keybindings
}

/// Insert and normal keymaps for vi mode. Ctrl-U clears the draft in both,
/// since Esc leaves insert mode there.
pub(super) fn vi_keybindings() -> (Keybindings, Keybindings) {
    let clear = ReedlineEvent::Edit(vec![EditCommand::Clear]);
    let mut insert = default_vi_insert_keybindings();
    add_chat_bindings(&mut insert);
    insert.add_binding(KeyModifiers::CONTROL, KeyCode::Char('u'), clear.clone());
    let mut normal = default_vi_normal_keybindings();
    normal.add_binding(
        KeyModifiers::CONTROL,
        KeyCode::Char('g'),
        ReedlineEvent::OpenEditor,
    );
    normal.add_binding(KeyModifiers::CONTROL, KeyCode::Char('u'), clear);
    (insert, normal)
}

pub(super) fn edit_mode(mode: EditModeConfig) -> ChatEditMode {
    let inner: Box<dyn EditMode> = match mode {
        EditModeConfig::Emacs => Box::new(Emacs::new(emacs_keybindings())),
        EditModeConfig::Vi => {
            let (insert, normal) = vi_keybindings();
            Box::new(Vi::new(insert, normal, default_vi_visual_keybindings()))
        }
    };
    ChatEditMode {
        inner,
        esc_clears: esc_clears_draft(mode),
        esc_pending: false,
    }
}

pub(super) fn build_editor(
    store_dir: &Path,
    mode: EditModeConfig,
) -> Result<Reedline, Box<dyn std::error::Error>> {
    let history = FileBackedHistory::with_file(HISTORY_CAPACITY, store_dir.join(HISTORY_FILE))?;
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());
    Ok(Reedline::create()
        .with_history(Box::new(history))
        .with_edit_mode(Box::new(edit_mode(mode)))
        .with_buffer_editor(Command::new(editor), store_dir.join(EDITOR_FILE))
        .use_bracketed_paste(true))
}
