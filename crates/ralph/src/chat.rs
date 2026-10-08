use crate::chat_memory::Store;
use crate::cli::ChatArgs;
use crate::highlight::ThemeConfig;
use crate::invoke::build_chat_command;
use crate::iteration::extract_response_text;
use crate::markdown::create_markdown_skin;
use crate::signal;
use crate::spinner::SpinnerSessionInfo;
use crate::stream_processor::VerboseToolsConfig;
use crate::subprocess::{
    invoke_subprocess_with_spinner_config, SpinnerSubprocessConfig, SubprocessError,
};
use crate::warn::{warn, warn_if_err};
use ralph_core::chat::{build_mcp_args, turn_prompt, SYSTEM_PROMPT, VIEW_LINES};
use ralph_core::chat_memory::{view, Role};
use reedline::{
    default_emacs_keybindings, DefaultPrompt, DefaultPromptSegment, EditCommand, Emacs,
    FileBackedHistory, KeyCode, KeyModifiers, Reedline, ReedlineEvent, Signal,
};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const PROMPT: &str = "> ";
const HISTORY_FILE: &str = "history.txt";
const EDITOR_FILE: &str = "editor.md";
const HISTORY_CAPACITY: usize = 1000;

fn default_store() -> io::Result<PathBuf> {
    dirs::home_dir()
        .map(|home| home.join(".ralph").join("chat"))
        .ok_or_else(|| io::Error::other("home directory not found"))
}

fn build_editor(store_dir: &Path) -> Result<Reedline, Box<dyn std::error::Error>> {
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
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());
    Ok(Reedline::create()
        .with_history(Box::new(history))
        .with_edit_mode(Box::new(Emacs::new(keybindings)))
        .with_buffer_editor(Command::new(editor), store_dir.join(EDITOR_FILE)))
}

fn next_message(editor: &mut Reedline) -> io::Result<Option<String>> {
    let prompt = DefaultPrompt::new(
        DefaultPromptSegment::Basic(PROMPT.to_string()),
        DefaultPromptSegment::Empty,
    );
    Ok(match editor.read_line(&prompt)? {
        Signal::Success(line) => Some(line.trim().to_string()).filter(|line| !line.is_empty()),
        _ => None,
    })
}

fn print_rule() {
    print!("{}", create_markdown_skin().term_text("---"));
}

fn spawn_nap(store: &Path) -> io::Result<()> {
    Command::new(std::env::current_exe()?)
        .arg("chat-nap")
        .arg("--store")
        .arg(store)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
}

struct Turn<'a> {
    store: &'a Store,
    store_dir: &'a Path,
    exe: &'a Path,
    timeout_secs: u64,
    theme_config: &'a ThemeConfig,
}

impl Turn<'_> {
    fn run(&self, message: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let (entries, tree) = self.store.load_settled()?;
        let lines = view(&entries, &tree, VIEW_LINES).lines;
        self.store.append(Role::User, message)?;
        let command = build_chat_command(
            &turn_prompt(&lines, message),
            SYSTEM_PROMPT,
            &build_mcp_args(self.exe, self.store_dir),
            dirs::home_dir().as_deref(),
        );
        print_rule();
        let result = invoke_subprocess_with_spinner_config(&SpinnerSubprocessConfig {
            command,
            timeout_secs: self.timeout_secs,
            theme_config: self.theme_config.clone(),
            session_elapsed_ms: 0,
            verbose_tools: VerboseToolsConfig::new(),
            session_info: SpinnerSessionInfo {
                persona: Some("chat".into()),
                slug: None,
                current_iteration: None,
                max_iterations: None,
                on_behalf_of: None,
            },
            pause: None,
            cancel: None,
        });
        let result = match result {
            Ok(result) => result,
            Err(SubprocessError::Interrupted { .. }) => return Ok(false),
            Err(e) => return Err(e.into()),
        };
        print_rule();
        match extract_response_text(&result.stream_result.output_blocks) {
            Some(reply) if result.exit_code == 0 => {
                self.store.append(Role::Assistant, &reply)?;
                warn_if_err(spawn_nap(self.store_dir), "Failed to start chat memory nap");
            }
            _ => warn(format!("chat turn failed (exit {})", result.exit_code)),
        }
        Ok(true)
    }
}

pub fn run(args: ChatArgs) -> Result<(), Box<dyn std::error::Error>> {
    warn_if_err(signal::init(), "Failed to initialize signal handler");
    let store_dir = match args.store {
        Some(dir) => dir,
        None => default_store()?,
    };
    std::fs::create_dir_all(&store_dir)?;
    let store = Store::new(store_dir.clone());
    let exe = std::env::current_exe()?;
    let theme_config = ThemeConfig::from_config_and_env();
    let turn = Turn {
        store: &store,
        store_dir: &store_dir,
        exe: &exe,
        timeout_secs: args.timeout,
        theme_config: &theme_config,
    };
    let mut editor = build_editor(&store_dir)?;
    while let Some(message) = next_message(&mut editor)? {
        if !turn.run(&message)? {
            break;
        }
    }
    Ok(())
}
