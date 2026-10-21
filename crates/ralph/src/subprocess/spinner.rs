//! Subprocess invocation with spinner display and gap detection.
//!
//! This module handles subprocess invocation with interactive features:
//! - Animated spinner during wait periods
//! - Gap detection to show spinner when LLM is thinking
//!
//! `Ctrl+C` kills the child process via the cancel flag or the signal path (`signal::is_interrupted()`).

use super::kill_process_group;
use super::timeout::try_wait_child;
use super::types::{
    StreamingSubprocessResult, SubprocessError, DEFAULT_GAP_THRESHOLD_MS, EXIT_CODE_INTERRUPTED,
    EXIT_CODE_KILLED,
};
use crate::highlight::ThemeConfig;
use crate::signal;
use crate::spinner::{Spinner, SpinnerContext, SpinnerSessionInfo};
use crate::stream_processor::{StreamProcessor, VerboseToolsConfig};
use std::io::IsTerminal;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Configuration for subprocess invocation with spinner.
///
/// Groups parameters to keep function signatures under 5 arguments.
#[derive(Debug, Clone)]
pub struct SpinnerSubprocessConfig {
    /// The command string to execute.
    pub command: String,
    pub timeout_secs: Option<u64>,
    /// Configuration for syntax highlighting theme.
    pub theme_config: ThemeConfig,
    /// Accumulated time from previous iterations in this session.
    pub session_elapsed_ms: u64,
    /// Configuration for verbose tool output.
    pub verbose_tools: VerboseToolsConfig,
    /// Session metadata for spinner display (slug, iteration info).
    pub session_info: SpinnerSessionInfo,
    pub pause: Option<Arc<AtomicBool>>,
    pub cancel: Option<Arc<AtomicBool>>,
}

fn drain_stdout(readers: &Readers, processor: &mut StreamProcessor) {
    while let Ok(line_result) = readers.line_rx.try_recv() {
        if let Ok(line) = line_result {
            if let Some(output) = processor.process_line(&line) {
                print!("{}", output);
                let _ = io::stdout().flush();
            }
        }
    }
}

fn drain_stderr(readers: &Readers, captured: &mut String, show: bool) {
    while let Ok(line) = readers.stderr_rx.try_recv() {
        if show {
            eprintln!("{}", line);
        }
        captured.push_str(&line);
        captured.push('\n');
    }
}

fn result_is_error(line: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|value| value.get("is_error")?.as_bool())
        .unwrap_or(false)
}

/// Invokes a command with stream processing, theme configuration, and spinner display.
///
/// This extends [`super::invoke_subprocess_with_timeout`] with spinner support:
/// - Shows an animated spinner while waiting for LLM to respond
/// - Displays elapsed time updating every second
/// - Displays session name and iteration progress (if provided)
/// - Automatically stops spinner when first output arrives
/// - Only shows spinner when stdout is a terminal
///
/// # Arguments
///
/// * `config` - Configuration for the subprocess invocation
///
/// # Returns
///
/// Returns the subprocess result directly.
///
/// # Example
///
/// ```no_run
/// use ralph::subprocess::{invoke_subprocess_with_spinner_config, SpinnerSubprocessConfig};
/// use ralph::highlight::ThemeConfig;
/// use ralph::stream_processor::VerboseToolsConfig;
/// use ralph::spinner::SpinnerSessionInfo;
///
/// let config = SpinnerSubprocessConfig {
///     command: "claude --output-format stream-json -p 'hello'".to_string(),
///     timeout_secs: Some(300),
///     theme_config: ThemeConfig::new().with_theme("Monokai Extended"),
///     session_elapsed_ms: 0,
///     verbose_tools: VerboseToolsConfig::new(),
///     session_info: SpinnerSessionInfo::default(),
///     pause: None,
///     cancel: None,
/// };
/// let result = invoke_subprocess_with_spinner_config(&config);
/// println!("{result:?}");
/// ```
pub fn invoke_subprocess_with_spinner_config(
    config: &SpinnerSubprocessConfig,
) -> Result<StreamingSubprocessResult, SubprocessError> {
    run_subprocess_with_spinner(config)
}

pub(super) fn spawn_piped(command_text: &str, stdin: Stdio) -> io::Result<Child> {
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(command_text)
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    signal::default_sigttou_in(&mut command);
    command.spawn()
}

pub(super) struct Readers {
    line_rx: mpsc::Receiver<io::Result<String>>,
    stderr_rx: mpsc::Receiver<String>,
    threads: Vec<thread::JoinHandle<()>>,
}

impl Readers {
    pub(super) fn start(child: &mut Child) -> Result<Readers, SubprocessError> {
        let stdout = child.stdout.take().ok_or_else(|| {
            SubprocessError::OutputCaptureFailed("stdout not captured".to_string())
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            SubprocessError::OutputCaptureFailed("stderr not captured".to_string())
        })?;
        let (stderr_tx, stderr_rx) = mpsc::channel::<String>();
        let stderr_thread = thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if stderr_tx.send(line).is_err() {
                    break;
                }
            }
        });
        let (line_tx, line_rx) = mpsc::channel::<io::Result<String>>();
        let stdout_thread = thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if line_tx.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Readers {
            line_rx,
            stderr_rx,
            threads: vec![stdout_thread, stderr_thread],
        })
    }

    pub(super) fn take_queued(&self) -> Vec<String> {
        self.line_rx.try_iter().filter_map(Result::ok).collect()
    }

    fn join(&mut self) {
        self.threads.drain(..).for_each(|handle| {
            let _ = handle.join();
        });
    }
}

pub(super) enum Step {
    Pending,
    Result { is_error: bool },
    Closed,
}

pub(super) struct Pump {
    processor: StreamProcessor,
    spinner: Spinner,
    spinner_active: bool,
    last_output_time: Instant,
    pending_tool_count: usize,
    stderr: String,
}

impl Pump {
    pub(super) fn start(config: &SpinnerSubprocessConfig) -> Result<Pump, SubprocessError> {
        let stdout_is_terminal = std::io::stdout().is_terminal();
        let processor = StreamProcessor::with_verbose_tools(
            config.theme_config.clone(),
            stdout_is_terminal,
            stdout_is_terminal,
            config.verbose_tools.clone(),
        )?;
        let mut spinner =
            Spinner::with_session_context(config.session_elapsed_ms, config.session_info.clone());
        spinner.start();
        Ok(Pump {
            processor,
            spinner,
            spinner_active: true,
            last_output_time: Instant::now(),
            pending_tool_count: 0,
            stderr: String::new(),
        })
    }

    fn stop_spinner(&mut self) {
        if self.spinner_active {
            self.spinner.stop();
            self.spinner_active = false;
        }
    }

    pub(super) fn step(
        &mut self,
        readers: &Readers,
        paused: bool,
    ) -> Result<Step, SubprocessError> {
        if paused {
            self.stop_spinner();
        }
        match readers.line_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(line)) => {
                if line.contains("\"type\":\"tool_use\"") {
                    self.pending_tool_count += 1;
                }
                if line.contains("\"type\":\"user\"") && line.contains("\"type\":\"tool_result\"") {
                    self.pending_tool_count = self.pending_tool_count.saturating_sub(1);
                }
                if let Some(output) = self.processor.process_line(&line) {
                    self.stop_spinner();
                    print!("{}", output);
                    let _ = io::stdout().flush();
                    self.last_output_time = Instant::now();
                }
                if line.contains("\"type\":\"result\"") {
                    return Ok(Step::Result {
                        is_error: result_is_error(&line),
                    });
                }
                Ok(Step::Pending)
            }
            Ok(Err(e)) => {
                self.stop_spinner();
                Err(SubprocessError::OutputCaptureFailed(format!(
                    "Failed to read stdout: {}",
                    e
                )))
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                drain_stderr(readers, &mut self.stderr, true);
                let gap_threshold = Duration::from_millis(DEFAULT_GAP_THRESHOLD_MS);
                if !paused
                    && !self.spinner_active
                    && self.last_output_time.elapsed() >= gap_threshold
                {
                    let context = if self.pending_tool_count > 0 {
                        SpinnerContext::WaitingForTool
                    } else {
                        SpinnerContext::Thinking
                    };
                    self.spinner.start_with_context(context);
                    self.spinner_active = true;
                }
                Ok(Step::Pending)
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Ok(Step::Closed),
        }
    }

    pub(super) fn finish(
        mut self,
        readers: &Readers,
        exit_code: i32,
        show_stderr: bool,
    ) -> StreamingSubprocessResult {
        self.stop_spinner();
        drain_stdout(readers, &mut self.processor);
        drain_stderr(readers, &mut self.stderr, show_stderr);
        let stream_result = self.processor.finish();
        if let Some(ref output) = stream_result.final_output {
            print!("{}", output);
            let _ = io::stdout().flush();
        }
        StreamingSubprocessResult {
            exit_code,
            stderr: self.stderr,
            stream_result,
        }
    }
}

fn run_subprocess_with_spinner(
    config: &SpinnerSubprocessConfig,
) -> Result<StreamingSubprocessResult, SubprocessError> {
    let mut child = spawn_piped(&config.command, Stdio::null())?;
    let mut readers = Readers::start(&mut child)?;
    let mut pump = Pump::start(config)?;
    let start = Instant::now();

    loop {
        if let Some(status) = try_wait_child(&mut child)? {
            let exit_code = status.code().ok_or(SubprocessError::Signaled)?;
            readers.join();
            return Ok(pump.finish(&readers, exit_code, true));
        }

        let paused = config
            .pause
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::SeqCst));

        if let Some(timeout_secs) = config
            .timeout_secs
            .filter(|secs| start.elapsed() >= Duration::from_secs(*secs))
        {
            pump.stop_spinner();
            kill_process_group(&mut child);
            readers.join();
            return Err(SubprocessError::Timeout {
                timeout_secs,
                partial_result: Box::new(pump.finish(&readers, EXIT_CODE_KILLED, false)),
            });
        }

        let cancelled = config
            .cancel
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::SeqCst));
        if cancelled || signal::is_interrupted() {
            pump.stop_spinner();
            kill_process_group(&mut child);
            readers.join();
            let partial = pump.finish(&readers, EXIT_CODE_INTERRUPTED, false);
            if cancelled {
                return Ok(partial);
            }
            return Err(SubprocessError::Interrupted {
                partial_result: Box::new(partial),
            });
        }

        pump.step(&readers, paused)?;
    }
}
