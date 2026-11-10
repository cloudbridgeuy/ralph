use crate::chat::monitor::Monitor;
use crate::chat::suggestion::SuggestionHighlighter;
use crate::chat::thread::{fork_of, open_names};
use crate::clipboard::PASTE_IMAGE;
use crate::render::image::resolve;
use crate::signal::default_sigttou_in;
use ralph_core::chat_direct::direct_completions;
use ralph_core::chat_draft::{draft_below_separator, editor_prefill};
use ralph_core::chat_edit_mode::{esc_clears_draft, EditMode as EditModeConfig};
use ralph_core::chat_hold::{hold_completions, HoldCompletion};
use ralph_core::chat_memory::Role;
use ralph_core::chat_nudge::nudge_completions;
use ralph_core::chat_switch::switch_completions;
use ralph_core::chat_view::{view_path_completion, view_path_query, DirEntry};
use ralph_server::chat::memory::Store;
use reedline::{
    default_emacs_keybindings, default_vi_insert_keybindings, default_vi_normal_keybindings,
    default_vi_visual_keybindings, ColumnarMenu, Completer, CompletionResult, EditCommand,
    EditMode, Emacs, FileBackedHistory, KeyCode, KeyModifiers, Keybindings, MenuBuilder,
    PromptEditMode, PromptViMode, Reedline, ReedlineEvent, ReedlineMenu, ReedlineRawEvent, Span,
    Suggestion, Vi,
};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

pub(super) const OPEN_EDITOR: &str = "open-editor";
pub(super) const BACK_OUT: &str = "back-out-queue";
pub(super) const QUEUE_CTRL_C: &str = "queue-ctrl-c";
const HISTORY_FILE: &str = "history.txt";
const EDITOR_FILE: &str = "editor.md";
const HISTORY_CAPACITY: usize = 1000;
const COMPLETION_MENU: &str = "completion_menu";

struct HoldCompleter {
    monitor: Arc<Monitor>,
    store: PathBuf,
}

impl Completer for HoldCompleter {
    fn complete(&mut self, line: &str, pos: usize) -> CompletionResult {
        let status = self.monitor.status();
        let suggestions: Vec<Suggestion> = hold_completions(line, pos, &status)
            .or_else(|| nudge_completions(line, pos, &status))
            .or_else(|| direct_completions(line, pos, &status))
            .or_else(|| view_completions(line, pos))
            .or_else(|| self.thread_completions(line, pos))
            .map(|completion| {
                let span = Span::new(completion.span.start, completion.span.end);
                completion
                    .values
                    .into_iter()
                    .map(|value| Suggestion {
                        value,
                        span,
                        ..Suggestion::default()
                    })
                    .collect()
            })
            .unwrap_or_default();
        CompletionResult::fresh(suggestions)
    }
}

impl HoldCompleter {
    fn thread_completions(&self, line: &str, pos: usize) -> Option<HoldCompletion> {
        let (main, current) = match fork_of(&self.store) {
            Some((main, name)) => (main, Some(name)),
            None => (self.store.clone(), None),
        };
        switch_completions(line, pos, current.as_ref(), &open_names(&main))
    }
}

fn view_completions(line: &str, pos: usize) -> Option<HoldCompletion> {
    let query = view_path_query(line, pos)?;
    let dir = match query.dir.as_str() {
        "" => Path::new(".").to_path_buf(),
        typed => resolve(typed),
    };
    let entries: Vec<DirEntry> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| DirEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            is_dir: std::fs::metadata(entry.path()).is_ok_and(|meta| meta.is_dir()),
        })
        .collect();
    Some(view_path_completion(&query, &entries))
}

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
    keybindings.add_binding(
        KeyModifiers::NONE,
        KeyCode::Tab,
        ReedlineEvent::UntilFound(vec![
            ReedlineEvent::Menu(COMPLETION_MENU.to_string()),
            ReedlineEvent::MenuNext,
        ]),
    );
    keybindings.add_binding(
        KeyModifiers::SHIFT,
        KeyCode::BackTab,
        ReedlineEvent::MenuPrevious,
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
    keybindings.add_binding(
        KeyModifiers::CONTROL,
        KeyCode::Char('q'),
        ReedlineEvent::ExecuteHostCommand(BACK_OUT.to_string()),
    );
    keybindings.add_binding(
        KeyModifiers::CONTROL,
        KeyCode::Char('c'),
        ReedlineEvent::ExecuteHostCommand(QUEUE_CTRL_C.to_string()),
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
    highlighter: Option<SuggestionHighlighter>,
    monitor: Arc<Monitor>,
) -> Result<Reedline, Box<dyn std::error::Error>> {
    let history = FileBackedHistory::with_file(HISTORY_CAPACITY, store_dir.join(HISTORY_FILE))?;
    let editor = Reedline::create()
        .with_history(Box::new(history))
        .with_edit_mode(Box::new(edit_mode(mode)))
        .with_completer(Box::new(HoldCompleter {
            monitor,
            store: store_dir.to_path_buf(),
        }))
        .with_menu(ReedlineMenu::EngineCompleter(Box::new(
            ColumnarMenu::default().with_name(COMPLETION_MENU),
        )))
        .use_bracketed_paste(true);
    Ok(match highlighter {
        Some(highlighter) => editor.with_highlighter(Box::new(highlighter)),
        None => editor,
    })
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
    let (entries, _) = Store::new(store_dir.to_path_buf()).load()?;
    let reply = entries
        .iter()
        .rev()
        .find(|entry| entry.role == Role::Assistant)
        .map_or("", |entry| entry.text.as_str());
    let prefill = editor_prefill(reply, editor.current_buffer_contents());
    edit_text(editor, store_dir, argv, &prefill)
}

pub(super) fn edit_queue_draft(
    editor: &mut Reedline,
    store_dir: &Path,
    argv: &[String],
) -> io::Result<()> {
    let draft = editor.current_buffer_contents().to_string();
    edit_text(editor, store_dir, argv, &draft)
}

fn edit_text(
    editor: &mut Reedline,
    store_dir: &Path,
    argv: &[String],
    prefill: &str,
) -> io::Result<()> {
    let path = store_dir.join(EDITOR_FILE);
    std::fs::write(&path, prefill)?;
    open_in_editor(argv, &path)?;
    let draft = draft_below_separator(&std::fs::read_to_string(&path)?);
    if !draft.is_empty() {
        editor.run_edit_commands(&[EditCommand::Clear, EditCommand::InsertString(draft)]);
    }
    Ok(())
}

pub(super) fn open_in_editor(argv: &[String], path: &Path) -> io::Result<()> {
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| io::Error::other("editor command is empty"))?;
    let mut command = Command::new(program);
    command.args(args).arg(path);
    default_sigttou_in(&mut command);
    let status = command.status()?;
    if !status.success() {
        return Err(io::Error::other(format!("editor exited with {status}")));
    }
    Ok(())
}
