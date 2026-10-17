use super::editor::{
    edit_mode, emacs_keybindings, esc_esc, vi_keybindings, visual_reset, OPEN_EDITOR,
};
use super::resolve_store;
use super::turn::{draft_after, logged_before_reply, rule, shown_output, TurnOutcome};
use crate::stream_processor::OutputBlock;
use crossterm::event::{Event, KeyEvent};
use ralph_core::chat_edit_mode::EditMode as EditModeConfig;
use ralph_core::chat_memory::Trigger;
use reedline::{
    EditCommand, EditMode, KeyCode, KeyModifiers, PromptEditMode, PromptViMode, ReedlineEvent,
    ReedlineRawEvent,
};
use std::path::{Path, PathBuf};
use termimad::MadSkin;

#[test]
fn only_worker_turns_log_before_the_reply() {
    assert!(logged_before_reply(Trigger::Worker));
    assert!(!logged_before_reply(Trigger::User));
}

#[test]
fn unanswered_turn_restores_the_draft() {
    assert_eq!(
        draft_after(TurnOutcome::Unanswered, "hi".into()),
        Some("hi".into())
    );
    assert_eq!(draft_after(TurnOutcome::Replied, "hi".into()), None);
    assert_eq!(draft_after(TurnOutcome::Exit, "hi".into()), None);
}

#[test]
fn rule_follows_shown_output_only() {
    assert!(!shown_output(&[]));
    assert!(shown_output(&[OutputBlock::Separator]));
}

#[test]
fn rule_spans_the_given_width() {
    let line = rule(&MadSkin::no_style(), 20);
    assert_eq!(line.trim_end().chars().count(), 20);
}

#[test]
fn second_esc_clears_the_draft() {
    let (pending, first) = esc_esc(false, ReedlineEvent::Esc);
    assert!(pending && first == ReedlineEvent::Esc);
    let (pending, second) = esc_esc(pending, ReedlineEvent::Esc);
    assert!(!pending && second == ReedlineEvent::Edit(vec![EditCommand::Clear]));
}

#[test]
fn other_event_resets_pending_esc() {
    let (pending, event) = esc_esc(true, ReedlineEvent::Enter);
    assert!(!pending && event == ReedlineEvent::Enter);
}

#[test]
fn emacs_keymap_keeps_chat_bindings() {
    let keys = emacs_keybindings();
    assert_eq!(
        keys.find_binding(KeyModifiers::ALT, KeyCode::Enter),
        Some(ReedlineEvent::Edit(vec![EditCommand::InsertNewline]))
    );
    assert_eq!(
        keys.find_binding(KeyModifiers::CONTROL, KeyCode::Char('o')),
        Some(ReedlineEvent::Edit(vec![EditCommand::Redo]))
    );
}

#[test]
fn vi_insert_keymap_carries_chat_bindings() {
    let (insert, _) = vi_keybindings();
    let newline = Some(ReedlineEvent::Edit(vec![EditCommand::InsertNewline]));
    assert_eq!(
        insert.find_binding(KeyModifiers::ALT, KeyCode::Enter),
        newline
    );
    assert_eq!(
        insert.find_binding(KeyModifiers::CONTROL, KeyCode::Char('j')),
        newline
    );
    assert!(matches!(
        insert.find_binding(KeyModifiers::CONTROL, KeyCode::Char('v')),
        Some(ReedlineEvent::ExecuteHostCommand(_))
    ));
    assert_eq!(
        insert.find_binding(KeyModifiers::CONTROL, KeyCode::Char('o')),
        Some(ReedlineEvent::Edit(vec![EditCommand::Redo]))
    );
}

#[test]
fn vi_ctrl_u_clears_the_draft_in_both_modes() {
    let (insert, normal) = vi_keybindings();
    let clear = Some(ReedlineEvent::Edit(vec![EditCommand::Clear]));
    assert_eq!(
        insert.find_binding(KeyModifiers::CONTROL, KeyCode::Char('u')),
        clear
    );
    assert_eq!(
        normal.find_binding(KeyModifiers::CONTROL, KeyCode::Char('u')),
        clear
    );
    assert_eq!(
        normal.find_binding(KeyModifiers::CONTROL, KeyCode::Char('g')),
        Some(ReedlineEvent::ExecuteHostCommand(OPEN_EDITOR.to_string()))
    );
}

#[test]
fn only_visual_mode_is_reset_to_normal() {
    assert_eq!(
        visual_reset(PromptEditMode::Vi(PromptViMode::Visual)),
        Some(ReedlineEvent::SwitchMode(PromptEditMode::Vi(
            PromptViMode::Normal
        )))
    );
    assert_eq!(visual_reset(PromptEditMode::Vi(PromptViMode::Normal)), None);
    assert_eq!(visual_reset(PromptEditMode::Vi(PromptViMode::Insert)), None);
    assert_eq!(visual_reset(PromptEditMode::Emacs), None);
}

fn press(mode: &mut impl EditMode, code: KeyCode, modifiers: KeyModifiers) -> ReedlineEvent {
    let event = ReedlineRawEvent::try_from(Event::Key(KeyEvent::new(code, modifiers))).unwrap();
    mode.parse_event(event)
}

#[test]
fn vi_v_and_shift_v_stay_in_normal_mode() {
    let mut mode = edit_mode(EditModeConfig::Vi);
    press(&mut mode, KeyCode::Esc, KeyModifiers::NONE);
    let normal = PromptEditMode::Vi(PromptViMode::Normal);
    assert_eq!(mode.edit_mode(), normal);
    let v = press(&mut mode, KeyCode::Char('v'), KeyModifiers::NONE);
    assert_eq!((v, mode.edit_mode()), (ReedlineEvent::None, normal.clone()));
    press(&mut mode, KeyCode::Char('V'), KeyModifiers::SHIFT);
    assert_eq!(mode.edit_mode(), normal);
}

#[test]
fn vi_find_v_still_moves() {
    let mut mode = edit_mode(EditModeConfig::Vi);
    press(&mut mode, KeyCode::Esc, KeyModifiers::NONE);
    press(&mut mode, KeyCode::Char('f'), KeyModifiers::NONE);
    let event = press(&mut mode, KeyCode::Char('v'), KeyModifiers::NONE);
    assert_ne!(event, ReedlineEvent::None);
    assert_eq!(mode.edit_mode(), PromptEditMode::Vi(PromptViMode::Normal));
}

#[test]
fn store_precedence_is_flag_then_env_then_home() {
    let cwd = Path::new("/work");
    let home = Some(Path::new("/home/u"));
    let path = |s: &str| Some(PathBuf::from(s));
    assert_eq!(
        resolve_store(path("/f"), path("/e"), cwd, home).unwrap(),
        PathBuf::from("/f")
    );
    assert_eq!(
        resolve_store(None, path("/e"), cwd, home).unwrap(),
        PathBuf::from("/e")
    );
    assert_eq!(
        resolve_store(None, None, cwd, home).unwrap(),
        PathBuf::from("/home/u/.ralph/chat")
    );
}

#[test]
fn relative_store_joins_cwd() {
    let cwd = Path::new("/work");
    let path = |s: &str| Some(PathBuf::from(s));
    assert_eq!(
        resolve_store(path("a"), None, cwd, None).unwrap(),
        PathBuf::from("/work/a")
    );
    assert_eq!(
        resolve_store(None, path("b"), cwd, None).unwrap(),
        PathBuf::from("/work/b")
    );
}

#[test]
fn empty_env_store_is_ignored_and_missing_home_errors() {
    let cwd = Path::new("/work");
    let empty = Some(PathBuf::new());
    assert_eq!(
        resolve_store(None, empty.clone(), cwd, Some(Path::new("/h"))).unwrap(),
        PathBuf::from("/h/.ralph/chat")
    );
    assert!(resolve_store(None, empty, cwd, None).is_err());
}
