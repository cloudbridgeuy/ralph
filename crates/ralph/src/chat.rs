use crate::chat_memory::Store;
use crate::cli::ChatArgs;
use crate::highlight::ThemeConfig;
use crate::invoke::build_chat_command;
use crate::iteration::extract_response_text;
use crate::signal;
use crate::spinner::SpinnerSessionInfo;
use crate::stream_processor::VerboseToolsConfig;
use crate::subprocess::{
    invoke_subprocess_with_spinner_config, SpinnerSubprocessConfig, SubprocessError,
};
use crate::warn::{warn, warn_if_err};
use ralph_core::chat::{build_mcp_args, turn_prompt, SYSTEM_PROMPT, VIEW_LINES};
use ralph_core::chat_memory::{view, Role};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

const INPUT_POLL: Duration = Duration::from_millis(100);
const PROMPT: &str = "> ";

fn default_store() -> io::Result<PathBuf> {
    dirs::home_dir()
        .map(|home| home.join(".ralph").join("chat"))
        .ok_or_else(|| io::Error::other("home directory not found"))
}

fn spawn_input() -> Receiver<String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        while matches!(io::stdin().lock().read_line(&mut line), Ok(n) if n > 0) {
            if tx.send(line.trim().to_string()).is_err() {
                return;
            }
            line.clear();
        }
    });
    rx
}

fn next_message(input: &Receiver<String>) -> Option<String> {
    print!("{PROMPT}");
    let _ = io::stdout().flush();
    loop {
        match input.recv_timeout(INPUT_POLL) {
            Ok(line) => return Some(line).filter(|line| !line.is_empty()),
            Err(RecvTimeoutError::Timeout) if signal::is_interrupted() => return None,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return None,
        }
    }
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
        println!();
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
    let input = spawn_input();
    while let Some(message) = next_message(&input) {
        if !turn.run(&message)? {
            break;
        }
    }
    Ok(())
}
