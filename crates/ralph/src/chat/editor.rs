use crate::chat::memory::Store;
use crate::clipboard::PASTE_IMAGE;
use crate::signal::default_sigttou_in;
use ralph_core::chat_draft::{draft_below_separator, editor_prefill};
use ralph_core::chat_edit_mode::{esc_clears_draft, EditMode as EditModeConfig};
use ralph_core::chat_memory::Role;
use reedline::{
    default_emacs_keybindings, default_vi_insert_keybindings, default_vi_normal_keybindings,
    default_vi_visual_keybindings, EditCommand, EditMode, Emacs, FileBackedHistory, KeyCode,
    KeyModifiers, Keybindings, PromptEditMode, PromptViMode, Reedline, ReedlineEvent,
    ReedlineRawEvent, Vi,
};
use std::io;
use std::path::Path;
use std::process::Command;

pub(super) const OPEN_EDITOR: &str = "open-editor";
pub(super) const CLOSE_QUEUE: &str = "close-queue";
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
        ReedlineEvent::ExecuteHostCommand(OPEN_EDITOR.to_string()),
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
        ReedlineEvent::ExecuteHostCommand(OPEN_EDITOR.to_string()),
    );
    normal.add_binding(KeyModifiers::CONTROL, KeyCode::Char('u'), clear);
    (insert, normal)
}

pub(super) fn edit_mode(mode: EditModeConfig) -> ChatEditMode {
    edit_mode_with(mode, |_| {})
}

pub(super) fn queue_bindings(keybindings: &mut Keybindings) {
    keybindings.remove_binding(KeyModifiers::CONTROL, KeyCode::Char('v'));
    keybindings.remove_binding(KeyModifiers::CONTROL, KeyCode::Char('g'));
    keybindings.add_binding(
        KeyModifiers::CONTROL,
        KeyCode::Char('q'),
        ReedlineEvent::ExecuteHostCommand(CLOSE_QUEUE.to_string()),
    );
}

fn edit_mode_with(mode: EditModeConfig, extra: fn(&mut Keybindings)) -> ChatEditMode {
    let inner: Box<dyn EditMode> = match mode {
        EditModeConfig::Emacs => {
            let mut keybindings = emacs_keybindings();
            extra(&mut keybindings);
            Box::new(Emacs::new(keybindings))
        }
        EditModeConfig::Vi => {
            let (mut insert, mut normal) = vi_keybindings();
            extra(&mut insert);
            extra(&mut normal);
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
    Ok(Reedline::create()
        .with_history(Box::new(history))
        .with_edit_mode(Box::new(edit_mode(mode)))
        .use_bracketed_paste(true))
}

pub(super) fn build_queue_editor(mode: EditModeConfig) -> Reedline {
    Reedline::create()
        .with_edit_mode(Box::new(edit_mode_with(mode, queue_bindings)))
        .use_bracketed_paste(true)
}

pub(super) fn edit_draft(
    editor: &mut Reedline,
    store_dir: &Path,
    argv: &[String],
) -> io::Result<()> {
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| io::Error::other("editor command is empty"))?;
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
    let mut command = Command::new(program);
    command.args(args).arg(&path);
    default_sigttou_in(&mut command);
    let status = command.status()?;
    if !status.success() {
        return Err(io::Error::other(format!("editor exited with {status}")));
    }
    let draft = draft_below_separator(&std::fs::read_to_string(&path)?);
    if !draft.is_empty() {
        editor.run_edit_commands(&[EditCommand::Clear, EditCommand::InsertString(draft)]);
    }
    Ok(())
}
