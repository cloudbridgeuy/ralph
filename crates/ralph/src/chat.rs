use crate::chat_memory::Store;
use crate::cli::ChatArgs;
use crate::config::AppConfig;
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
use crate::worker_mcp::Inbox;
use ralph_core::chat::{
    build_agents_args, build_mcp_args, mcp_config, select_servers, turn_prompt, SYSTEM_PROMPT,
    VIEW_LINES,
};
use ralph_core::chat_memory::{usage_record, view, CallKind, Role, Trigger};
use ralph_core::worker::{chat_message, INBOX_DIR};
use reedline::{
    default_emacs_keybindings, EditCommand, Emacs, FileBackedHistory, KeyCode, KeyModifiers,
    Prompt, PromptEditMode, PromptHistorySearch, PromptHistorySearchStatus, Reedline,
    ReedlineEvent, Signal, DEFAULT_MULTILINE_INDICATOR,
};
use std::borrow::Cow;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

const PROMPT: &str = ">>> ";
const HISTORY_FILE: &str = "history.txt";
const EDITOR_FILE: &str = "editor.md";
const HISTORY_CAPACITY: usize = 1000;
const INBOX_POLL: Duration = Duration::from_millis(500);

fn default_store() -> io::Result<PathBuf> {
    dirs::home_dir()
        .map(|home| home.join(".ralph").join("chat"))
        .ok_or_else(|| io::Error::other("home directory not found"))
}

fn build_editor(
    store_dir: &Path,
    mail: Arc<AtomicBool>,
) -> Result<Reedline, Box<dyn std::error::Error>> {
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
        .with_buffer_editor(Command::new(editor), store_dir.join(EDITOR_FILE))
        .with_break_signal(mail))
}

struct ChatPrompt;

impl Prompt for ChatPrompt {
    fn render_prompt_left(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn render_prompt_right(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn render_prompt_indicator(&self, _edit_mode: PromptEditMode) -> Cow<'_, str> {
        Cow::Borrowed(PROMPT)
    }

    fn render_prompt_multiline_indicator(&self) -> Cow<'_, str> {
        Cow::Borrowed(DEFAULT_MULTILINE_INDICATOR)
    }

    fn render_prompt_history_search_indicator(
        &self,
        history_search: PromptHistorySearch,
    ) -> Cow<'_, str> {
        let prefix = match history_search.status {
            PromptHistorySearchStatus::Passing => "",
            PromptHistorySearchStatus::Failing => "failing ",
        };
        Cow::Owned(format!(
            "({prefix}reverse-search: {}) ",
            history_search.term
        ))
    }
}

enum Input {
    Message(String),
    Mail,
    Quit,
}

fn next_input(editor: &mut Reedline) -> io::Result<Input> {
    Ok(match editor.read_line(&ChatPrompt)? {
        Signal::Success(line) if !line.trim().is_empty() => Input::Message(line.trim().to_string()),
        Signal::ExternalBreak(_) => Input::Mail,
        _ => Input::Quit,
    })
}

fn watch_inbox(dir: PathBuf, mail: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let inbox = Inbox::new(dir);
        loop {
            if inbox.has_mail() {
                mail.store(true, Ordering::Relaxed);
            }
            std::thread::sleep(INBOX_POLL);
        }
    });
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
    mcp_config_path: &'a Path,
    timeout_secs: u64,
    theme_config: &'a ThemeConfig,
}

impl Turn<'_> {
    fn run(&self, trigger: Trigger, message: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let (entries, tree) = self.store.load_settled()?;
        let lines = view(&entries, &tree, VIEW_LINES).lines;
        self.store.append(trigger.role(), message)?;
        let command = build_chat_command(
            &turn_prompt(&lines, message),
            SYSTEM_PROMPT,
            &[build_mcp_args(self.mcp_config_path), build_agents_args()].concat(),
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
            Err(SubprocessError::Timeout { timeout_secs, .. }) => {
                print_rule();
                warn(format!(
                    "chat turn timed out after {timeout_secs}s; restart with a larger --timeout for long work"
                ));
                return Ok(true);
            }
            Err(e) => return Err(e.into()),
        };
        if let Some(record) = usage_record(
            CallKind::Turn { trigger },
            chrono::Utc::now().to_rfc3339(),
            &result.stream_result.costs,
        ) {
            warn_if_err(
                self.store.append_usage(&record),
                "Failed to record chat usage",
            );
        }
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

fn write_mcp_config(store: &Store, store_dir: &Path, exe: &Path) -> io::Result<PathBuf> {
    let server_names = AppConfig::load()
        .map(|config| config.chat.mcp_servers)
        .unwrap_or_else(|e| {
            warn(format!("Failed to load config: {e}"));
            AppConfig::default().chat.mcp_servers
        });
    let claude_config = read_claude_config();
    let imported = select_servers(&claude_config, &server_names);
    for name in &imported.missing {
        warn(format!(
            "chat: MCP server \"{name}\" not found in ~/.claude.json; continuing without it"
        ));
    }
    store.write_mcp_config(&mcp_config(exe, store_dir, &imported.servers))
}

fn read_claude_config() -> serde_json::Value {
    let Some(path) = dirs::home_dir().map(|home| home.join(".claude.json")) else {
        return serde_json::json!({});
    };
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            warn(format!("Failed to parse {}: {e}", path.display()));
            serde_json::json!({})
        }),
        Err(e) if e.kind() == io::ErrorKind::NotFound => serde_json::json!({}),
        Err(e) => {
            warn(format!("Failed to read {}: {e}", path.display()));
            serde_json::json!({})
        }
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
    let mcp_config_path = write_mcp_config(&store, &store_dir, &exe)?;
    let theme_config = ThemeConfig::from_config_and_env();
    let turn = Turn {
        store: &store,
        store_dir: &store_dir,
        mcp_config_path: &mcp_config_path,
        timeout_secs: args.timeout,
        theme_config: &theme_config,
    };
    let inbox_dir = store_dir.join(INBOX_DIR);
    let inbox = Inbox::new(inbox_dir.clone());
    let mail = Arc::new(AtomicBool::new(false));
    watch_inbox(inbox_dir, Arc::clone(&mail));
    let mut editor = build_editor(&store_dir, mail)?;
    loop {
        let reports = inbox.drain()?;
        let (trigger, message) = if reports.is_empty() {
            match next_input(&mut editor)? {
                Input::Message(message) => (Trigger::User, message),
                Input::Mail => {
                    println!();
                    continue;
                }
                Input::Quit => break,
            }
        } else {
            let message = chat_message(&reports);
            println!("{message}");
            (Trigger::Worker, message)
        };
        if !turn.run(trigger, &message)? {
            break;
        }
    }
    Ok(())
}
