use super::editor::{
    edit_mode, emacs_keybindings, esc_esc, queue_bindings, vi_keybindings, visual_reset,
    IDLE_CTRL_C, OPEN_EDITOR,
};
use super::inbox::Display;
use super::{reply_style, resolve_store};
use crate::highlight::ThemeConfig;
use crate::markdown::Palette;
use crate::render::pill_style::PillStyle;
use crate::render::RenderContext;
use crate::stream_processor::BlockRenderer;
use crate::turn_sink::{rule, rule_skin};
use crate::ui_theme::UiColors;
use crossterm::event::{Event, KeyEvent};
use ralph_core::chat_edit_mode::EditMode as EditModeConfig;
use ralph_core::chat_help::{KeyHelp, PROMPT_KEYS, QUEUE_KEYS, VI_KEYS};
use ralph_core::chunk::ParsedChunk;
use ralph_core::stream::blocks::OutputBlock;
use reedline::{
    default_emacs_keybindings, default_vi_insert_keybindings, default_vi_normal_keybindings,
    EditCommand, EditMode, KeyCode, KeyModifiers, Keybindings, PromptEditMode, PromptViMode,
    ReedlineEvent, ReedlineRawEvent, Signal,
};
use std::path::{Path, PathBuf};
use termimad::crossterm::style::Color;
use termimad::MadSkin;

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

fn complete_on_tab() -> Option<ReedlineEvent> {
    Some(ReedlineEvent::UntilFound(vec![
        ReedlineEvent::Menu("completion_menu".to_string()),
        ReedlineEvent::MenuNext,
    ]))
}

#[test]
fn tab_completes_in_emacs_and_vi_insert_only() {
    let (insert, normal) = vi_keybindings();
    for keys in [emacs_keybindings(), insert] {
        assert_eq!(
            keys.find_binding(KeyModifiers::NONE, KeyCode::Tab),
            complete_on_tab()
        );
        assert_eq!(
            keys.find_binding(KeyModifiers::SHIFT, KeyCode::BackTab),
            Some(ReedlineEvent::MenuPrevious)
        );
    }
    assert_eq!(normal.find_binding(KeyModifiers::NONE, KeyCode::Tab), None);
    assert_eq!(
        normal.find_binding(KeyModifiers::SHIFT, KeyCode::BackTab),
        None
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
fn store_precedence_is_flag_then_env_then_cwd() {
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
        PathBuf::from("/work/.ralph")
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
fn empty_env_store_is_ignored_and_missing_home_still_resolves() {
    let cwd = Path::new("/work");
    let empty = Some(PathBuf::new());
    assert_eq!(
        resolve_store(None, empty.clone(), cwd, Some(Path::new("/h"))).unwrap(),
        PathBuf::from("/work/.ralph")
    );
    assert_eq!(
        resolve_store(None, empty, cwd, None).unwrap(),
        PathBuf::from("/work/.ralph")
    );
}

#[test]
fn default_store_refuses_the_runtime_dir_under_home() {
    let home = Some(Path::new("/home/u"));
    let path = |s: &str| Some(PathBuf::from(s));
    assert!(resolve_store(None, None, Path::new("/home/u"), home).is_err());
    assert_eq!(
        resolve_store(None, None, Path::new("/home/u/p"), home).unwrap(),
        PathBuf::from("/home/u/p/.ralph")
    );
    assert_eq!(
        resolve_store(path(".ralph"), None, Path::new("/home/u"), home).unwrap(),
        PathBuf::from("/home/u/.ralph")
    );
    assert_eq!(
        resolve_store(None, path("/home/u/.ralph"), Path::new("/home/u"), home).unwrap(),
        PathBuf::from("/home/u/.ralph")
    );
}

#[test]
fn queue_keymap_drops_paste_and_keeps_open_editor() {
    let mut keys = emacs_keybindings();
    queue_bindings(&mut keys);
    assert_eq!(
        keys.find_binding(KeyModifiers::CONTROL, KeyCode::Char('v')),
        None
    );
    assert_eq!(
        keys.find_binding(KeyModifiers::CONTROL, KeyCode::Char('g')),
        Some(ReedlineEvent::ExecuteHostCommand(OPEN_EDITOR.to_string()))
    );
    assert!(keys
        .find_binding(KeyModifiers::CONTROL, KeyCode::Char('q'))
        .is_some());
}

#[test]
fn queue_help_lists_ctrl_g_as_on() {
    let row = QUEUE_KEYS
        .iter()
        .find(|help| help.keys.split(", ").any(|key| key == "Ctrl-G"))
        .unwrap();
    assert!(!row.effect.starts_with("off"), "{}", row.effect);
}

fn key_label(modifier: KeyModifiers, code: KeyCode) -> String {
    let mut label = String::new();
    for (flag, name) in [
        (KeyModifiers::CONTROL, "Ctrl-"),
        (KeyModifiers::ALT, "Alt-"),
        (KeyModifiers::SHIFT, "Shift-"),
    ] {
        if modifier.contains(flag) {
            label.push_str(name);
        }
    }
    match code {
        KeyCode::Char(c) => label.push(c.to_ascii_uppercase()),
        KeyCode::BackTab => label.push_str("Tab"),
        other => label.push_str(&format!("{other:?}")),
    }
    label
}

/// Labels of the bindings ralph adds or changes on top of reedline's defaults.
fn added_labels(keys: &Keybindings, defaults: &Keybindings) -> Vec<String> {
    keys.get_keybindings()
        .iter()
        .filter(|(combo, event)| defaults.get_keybindings().get(*combo) != Some(*event))
        .map(|(combo, _)| key_label(combo.modifier, combo.key_code))
        .collect()
}

fn assert_documented(labels: Vec<String>, tables: &[&[KeyHelp]]) {
    for label in labels {
        let documented = tables
            .iter()
            .flat_map(|table| table.iter())
            .any(|help| help.keys.split(", ").any(|key| key == label));
        assert!(documented, "{label} is bound but has no /help entry");
    }
}

#[test]
fn every_chat_binding_has_a_help_entry() {
    assert_documented(
        added_labels(&emacs_keybindings(), &default_emacs_keybindings()),
        &[PROMPT_KEYS],
    );
    let (insert, normal) = vi_keybindings();
    assert_documented(
        added_labels(&insert, &default_vi_insert_keybindings()),
        &[PROMPT_KEYS, VI_KEYS],
    );
    assert_documented(
        added_labels(&normal, &default_vi_normal_keybindings()),
        &[PROMPT_KEYS, VI_KEYS],
    );
    let mut queue = emacs_keybindings();
    queue_bindings(&mut queue);
    assert_documented(
        added_labels(&queue, &default_emacs_keybindings()),
        &[QUEUE_KEYS, PROMPT_KEYS],
    );
}

#[test]
fn rule_with_the_default_color_keeps_todays_bytes() {
    let ui = crate::ui_theme::UiColors::default();
    assert_eq!(rule(&rule_skin(&ui), 20), rule(&MadSkin::default(), 20));
}

#[test]
fn rule_uses_the_configured_color() {
    let ui = crate::ui_theme::UiColors {
        rule: termimad::crossterm::style::Color::Rgb { r: 1, g: 2, b: 3 },
        ..crate::ui_theme::UiColors::default()
    };
    assert!(rule(&rule_skin(&ui), 4).contains("38;2;1;2;3"));
}

#[test]
fn idle_signals_map_to_the_close_signals() {
    use super::prompt::idle_signal;
    use ralph_core::chat_close::IdleSignal;
    assert_eq!(idle_signal(&Signal::CtrlC), Some(IdleSignal::CtrlC));
    assert_eq!(idle_signal(&Signal::CtrlD), Some(IdleSignal::CtrlD));
    assert_eq!(
        idle_signal(&Signal::HostCommand(IDLE_CTRL_C.into())),
        Some(IdleSignal::CtrlC)
    );
    assert_eq!(idle_signal(&Signal::HostCommand(OPEN_EDITOR.into())), None);
    assert_eq!(idle_signal(&Signal::ExternalBreak(String::new())), None);
}

fn styled_config(emphasis: bool) -> ThemeConfig {
    let palette = Palette {
        bold: Some(Color::Red),
        inline_code: Some(Color::Green),
        ..Palette::default()
    };
    ThemeConfig::default()
        .with_theme("Solarized (dark)")
        .with_markdown_style(palette, emphasis, PillStyle::resolve(true, true, emphasis))
}

fn reply_text(config: &ThemeConfig, text: &str) -> String {
    let (highlighter, theme) = reply_style(config);
    let display = Display {
        ctx: RenderContext::new(&highlighter, true),
        theme: &theme,
        color: true,
        ui: UiColors::default(),
    };
    super::view::render(&display, text)
}

fn streamed_text(config: &ThemeConfig, text: &str) -> String {
    BlockRenderer::new(Some(config.clone()), true)
        .unwrap()
        .render(&[OutputBlock::text(ParsedChunk::prose(text))])
}

const SAMPLE: &str = "# Title\n\n**bold** and `code` [[Done]]\n\n- item\n";

#[test]
fn reprint_matches_the_streamed_reply_theme() {
    let config = styled_config(false);
    assert_eq!(reply_text(&config, SAMPLE), streamed_text(&config, SAMPLE));
}

#[test]
fn reprint_keeps_emphasis_and_pill_style() {
    let config = styled_config(true);
    assert_eq!(reply_text(&config, SAMPLE), streamed_text(&config, SAMPLE));
}
