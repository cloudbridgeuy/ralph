use super::gate::OutputGate;
use super::timeout::try_wait_child;
use super::types::{
    StreamingSubprocessResult, SubprocessError, DEFAULT_GAP_THRESHOLD_MS, EXIT_CODE_INTERRUPTED,
    EXIT_CODE_KILLED,
};
use super::{default_sigttou_in, kill_process_group, Invocation};
use crate::disk_files::DiskFiles;
use ralph_core::cancel::CancelToken;
use ralph_core::stream::blocks::StreamCollector;
use ralph_core::stream::blocks::{OutputBlock, VerboseToolsConfig};
use ralph_core::stream::{is_main_assistant_line, result_is_error, ContentBlock, StreamEvent};
use ralph_core::turn_event::{TurnEvent, TurnSink, Wait};
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, Stdio};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct SpinnerSubprocessConfig {
    pub command: Invocation,
    pub timeout_secs: Option<u64>,
    pub verbose_tools: VerboseToolsConfig,
    pub show_tools: bool,
    pub terminated: fn() -> bool,
    pub sink: Arc<dyn TurnSink>,
    pub gate: Option<Arc<OutputGate>>,
    pub interrupt: Option<CancelToken>,
    pub stdin: Option<String>,
}

fn drain_stderr(readers: &Readers, captured: &mut String, sink: Option<&dyn TurnSink>) {
    while let Ok(line) = readers.stderr_rx.try_recv() {
        if let Some(sink) = sink {
            sink.emit(TurnEvent::Stderr(format!("{line}\n")));
        }
        captured.push_str(&line);
        captured.push('\n');
    }
}

pub fn invoke_subprocess_with_spinner_config(
    config: &SpinnerSubprocessConfig,
) -> Result<StreamingSubprocessResult, SubprocessError> {
    run_subprocess_with_spinner(config, Display::Shown, false)
}

pub fn invoke_subprocess_silent(
    config: &SpinnerSubprocessConfig,
    codex: bool,
) -> Result<StreamingSubprocessResult, SubprocessError> {
    run_subprocess_with_spinner(config, Display::Silent, codex)
}

pub(super) fn spawn_piped(invocation: &Invocation, stdin: Stdio) -> io::Result<Child> {
    let mut command = invocation.command();
    command
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    default_sigttou_in(&mut command);
    command
        .spawn()
        .map_err(|error| invocation.spawn_error(error))
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
    Control(Box<ralph_core::worker::ClaudeLine>, String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    Shown,
    Silent,
}

struct Shown {
    sink: Arc<dyn TurnSink>,
    waiting: bool,
}

impl Shown {
    fn wait(&mut self, event: TurnEvent) {
        self.sink.emit(event);
        self.waiting = true;
    }

    fn stop(&mut self) {
        if self.waiting {
            self.sink.emit(TurnEvent::Idle);
            self.waiting = false;
        }
    }

    fn show(&mut self, blocks: Vec<OutputBlock>) {
        self.stop();
        self.sink.emit(TurnEvent::Blocks(blocks));
    }
}

impl Drop for Shown {
    fn drop(&mut self) {
        self.stop();
    }
}

pub struct Pump {
    shown: Option<Shown>,
    collector: StreamCollector,
    codex: Option<ralph_core::worker::ExecStream>,
    last_output_time: Instant,
    pending_tool_count: usize,
    stderr: String,
    gate: Option<Arc<OutputGate>>,
    request_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl Pump {
    pub fn start(config: &SpinnerSubprocessConfig, display: Display) -> Pump {
        let collector = StreamCollector::new(config.show_tools, config.verbose_tools.clone());
        let shown = (display == Display::Shown).then(|| {
            let mut shown = Shown {
                sink: Arc::clone(&config.sink),
                waiting: false,
            };
            shown.wait(TurnEvent::Busy(Wait::Response));
            shown
        });
        Pump {
            shown,
            collector,
            codex: None,
            last_output_time: Instant::now(),
            pending_tool_count: 0,
            stderr: String::new(),
            gate: config.gate.clone(),
            request_at: None,
        }
    }

    fn stop_spinner(&mut self) {
        if let Some(shown) = self.shown.as_mut() {
            shown.stop();
        }
    }

    fn show(&mut self, blocks: Vec<OutputBlock>) {
        if let (false, Some(shown)) = (blocks.is_empty(), self.shown.as_mut()) {
            shown.show(blocks);
            self.last_output_time = Instant::now();
        }
    }

    pub fn event(&mut self, event: StreamEvent) {
        match &event {
            StreamEvent::Assistant(assistant) => {
                self.request_at = Some(chrono::Utc::now());
                let tool_uses = assistant
                    .message
                    .content
                    .iter()
                    .filter(|block| matches!(block, ContentBlock::ToolUse { .. }))
                    .count();
                self.pending_tool_count += tool_uses;
            }
            StreamEvent::User(user) => {
                self.pending_tool_count = self
                    .pending_tool_count
                    .saturating_sub(user.message.content.len());
            }
            _ => {}
        }
        let blocks = self.collector.process_event(event, &DiskFiles);
        self.show(blocks);
    }

    pub fn flush_prose(&mut self) {
        let blocks = self.collector.flush_streamed(&DiskFiles);
        self.show(blocks);
    }

    pub fn finish_events(mut self, exit_code: i32) -> StreamingSubprocessResult {
        self.stop_spinner();
        let (flushed, stream_result) = self.collector.finish(&DiskFiles);
        if let (false, Some(shown)) = (flushed.is_empty(), self.shown.as_mut()) {
            shown.show(flushed);
        }
        StreamingSubprocessResult {
            exit_code,
            stderr: String::new(),
            stream_result,
            request_at: self.request_at,
            cost_before: None,
            logged_before: None,
        }
    }

    pub(super) fn gate_paused(&self) -> bool {
        self.gate.as_ref().is_some_and(|gate| gate.is_paused())
    }

    pub(super) fn step(
        &mut self,
        readers: &Readers,
        paused: bool,
    ) -> Result<Step, SubprocessError> {
        if paused {
            self.stop_spinner();
            if let Some(gate) = &self.gate {
                gate.settle();
            }
        }
        match readers.line_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(line)) => {
                if let Some(codex) = self.codex.as_mut() {
                    codex.observe(&line);
                    return Ok(Step::Pending);
                }
                if let control @ (ralph_core::worker::ClaudeLine::Permission(_)
                | ralph_core::worker::ClaudeLine::Malformed
                | ralph_core::worker::ClaudeLine::Cancel(_)) =
                    ralph_core::worker::decode_line(&line)
                {
                    return Ok(Step::Control(Box::new(control), line));
                }
                if is_main_assistant_line(&line) {
                    self.request_at = Some(chrono::Utc::now());
                }
                if line.contains("\"type\":\"tool_use\"") {
                    self.pending_tool_count += 1;
                }
                if line.contains("\"type\":\"user\"") && line.contains("\"type\":\"tool_result\"") {
                    self.pending_tool_count = self.pending_tool_count.saturating_sub(1);
                }
                let blocks = self.collector.process_line(&line, &DiskFiles);
                self.show(blocks);
                if let Some(is_error) = result_is_error(&line) {
                    return Ok(Step::Result { is_error });
                }
                Ok(Step::Pending)
            }
            Ok(Err(e)) => {
                if let Some(codex) = self.codex.as_mut() {
                    codex.observe("invalid stdout frame");
                    return Ok(Step::Pending);
                }
                self.stop_spinner();
                Err(SubprocessError::OutputCaptureFailed(format!(
                    "Failed to read stdout: {}",
                    e
                )))
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let sink = self.shown.as_ref().map(|shown| shown.sink.as_ref());
                drain_stderr(readers, &mut self.stderr, sink);
                let gap_threshold = Duration::from_millis(DEFAULT_GAP_THRESHOLD_MS);
                let gap = self.last_output_time.elapsed() >= gap_threshold;
                let pending_tools = self.pending_tool_count;
                if let Some(shown) = self.shown.as_mut() {
                    if !paused && !shown.waiting && gap {
                        shown.wait(TurnEvent::Resume(Wait::after_gap(pending_tools)));
                    }
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
        if let Some(codex) = self.codex.as_mut() {
            for line in readers.line_rx.try_iter() {
                codex.observe(line.as_deref().unwrap_or("invalid stdout frame"));
            }
        }
        for line in readers.take_queued() {
            let blocks = self.collector.process_line(&line, &DiskFiles);
            if let (false, Some(shown)) = (blocks.is_empty(), self.shown.as_mut()) {
                shown.show(blocks);
            }
        }
        let sink = self.shown.as_ref().map(|shown| shown.sink.as_ref());
        drain_stderr(readers, &mut self.stderr, sink.filter(|_| show_stderr));
        let (flushed, stream_result) = self.collector.finish(&DiskFiles);
        if let (false, Some(shown)) = (flushed.is_empty(), self.shown.as_mut()) {
            shown.show(flushed);
        }
        let (stream_result, diagnostic) = match self.codex.take() {
            Some(codex) => codex.finish(),
            None => (stream_result, None),
        };
        if let Some(message) = diagnostic.as_ref() {
            if !self.stderr.is_empty() && !self.stderr.ends_with('\n') {
                self.stderr.push('\n');
            }
            self.stderr.push_str(message);
        }
        StreamingSubprocessResult {
            exit_code: if exit_code == 0 && diagnostic.is_some() {
                1
            } else {
                exit_code
            },
            stderr: self.stderr,
            stream_result,
            request_at: self.request_at,
            cost_before: None,
            logged_before: None,
        }
    }
}

type PromptWrite = thread::Result<io::Result<()>>;

fn prompt_write_warning(outcome: &PromptWrite) -> Option<String> {
    match outcome {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(format!(
            "claude stopped reading before the whole prompt was sent (short write): {error}"
        )),
        Err(_) => Some(
            "the prompt writer thread panicked; claude may have received a short prompt"
                .to_string(),
        ),
    }
}

fn report_prompt_write(writer: Option<thread::JoinHandle<io::Result<()>>>) {
    if let Some(message) = writer.and_then(|handle| prompt_write_warning(&handle.join())) {
        crate::warn::warn(message);
    }
}

fn run_subprocess_with_spinner(
    config: &SpinnerSubprocessConfig,
    display: Display,
    codex: bool,
) -> Result<StreamingSubprocessResult, SubprocessError> {
    let stdin = match config.stdin {
        Some(_) => Stdio::piped(),
        None => Stdio::null(),
    };
    let mut child = spawn_piped(&config.command, stdin)?;
    let writer = match (config.stdin.clone(), child.stdin.take()) {
        (Some(text), Some(mut pipe)) => {
            Some(thread::spawn(move || pipe.write_all(text.as_bytes())))
        }
        _ => None,
    };
    let mut readers = Readers::start(&mut child)?;
    let mut pump = Pump::start(config, display);
    pump.codex = codex.then(Default::default);
    let start = Instant::now();

    loop {
        if let Some(status) = try_wait_child(&mut child)? {
            let exit_code = status.code().ok_or(SubprocessError::Signaled)?;
            readers.join();
            report_prompt_write(writer);
            return Ok(pump.finish(&readers, exit_code, true));
        }

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

        let interrupted = config
            .interrupt
            .as_ref()
            .is_some_and(CancelToken::is_cancelled)
            || (config.terminated)();
        if interrupted {
            pump.stop_spinner();
            kill_process_group(&mut child);
            readers.join();
            return Err(SubprocessError::Interrupted {
                partial_result: Box::new(pump.finish(&readers, EXIT_CODE_INTERRUPTED, false)),
            });
        }

        pump.step(&readers, pump.gate_paused())?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::turn_event::{TurnEvent, TurnSink, Wait};
    use std::sync::Mutex;

    #[derive(Default)]
    struct Recording(Mutex<Vec<TurnEvent>>);

    impl TurnSink for Recording {
        fn emit(&self, event: TurnEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    impl Recording {
        fn events(&self) -> Vec<TurnEvent> {
            self.0.lock().unwrap().clone()
        }
    }

    const HELLO: &str = r#"{"type":"assistant","message":{"id":"msg-1","content":[{"type":"text","text":"Hello, world!"}]}}"#;

    fn config(sink: &Arc<Recording>) -> SpinnerSubprocessConfig {
        SpinnerSubprocessConfig {
            command: Invocation {
                program: "true",
                args: Vec::new(),
                env: Vec::new(),
            },
            timeout_secs: None,
            verbose_tools: VerboseToolsConfig::new(),
            show_tools: false,
            terminated: || false,
            sink: Arc::clone(sink) as Arc<dyn TurnSink>,
            gate: None,
            interrupt: None,
            stdin: None,
        }
    }

    struct Feed {
        _lines: mpsc::Sender<io::Result<String>>,
        _stderr: mpsc::Sender<String>,
    }

    fn feed(lines: &[&str], stderr: &[&str]) -> (Readers, Feed) {
        let (line_tx, line_rx) = mpsc::channel();
        let (stderr_tx, stderr_rx) = mpsc::channel();
        lines
            .iter()
            .for_each(|line| line_tx.send(Ok(line.to_string())).unwrap());
        stderr
            .iter()
            .for_each(|line| stderr_tx.send(line.to_string()).unwrap());
        let readers = Readers {
            line_rx,
            stderr_rx,
            threads: Vec::new(),
        };
        (
            readers,
            Feed {
                _lines: line_tx,
                _stderr: stderr_tx,
            },
        )
    }

    #[test]
    fn a_permission_request_reaches_the_session() {
        let sink = Arc::new(Recording::default());
        let (readers, _feed) = feed(
            &[
                r#"{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"mcp__mcptools__execute","input":{"code":"read"}}}"#,
            ],
            &[],
        );
        let mut pump = Pump::start(&config(&sink), Display::Silent);
        let Step::Control(control, line) = pump.step(&readers, false).unwrap() else {
            panic!("permission frame was not forwarded");
        };
        assert!(matches!(
            *control,
            ralph_core::worker::ClaudeLine::Permission(_)
        ));
        let reply = super::super::session::control_reply(*control, &line, false, None)
            .unwrap()
            .unwrap();
        let response: serde_json::Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(response["response"]["request_id"], "r1");
        assert_eq!(response["response"]["response"]["behavior"], "allow");
        assert_eq!(
            response["response"]["response"]["updatedInput"],
            serde_json::json!({"code":"read"})
        );
    }

    #[test]
    fn a_shown_pump_waits_then_stops_the_spinner_before_its_output() {
        let sink = Arc::new(Recording::default());
        let (readers, _feed) = feed(&[HELLO], &[]);
        let mut pump = Pump::start(&config(&sink), Display::Shown);
        pump.step(&readers, false).unwrap();
        pump.finish(&readers, 0, true);
        let events = sink.events();
        assert_eq!(events.first(), Some(&TurnEvent::Busy(Wait::Response)));
        let output = events
            .iter()
            .position(|event| matches!(event, TurnEvent::Blocks(blocks) if blocks.iter().any(|block| matches!(block, OutputBlock::Text(text) if text.chunk.content.contains("Hello")))))
            .expect("a Blocks event");
        let idle = events.iter().position(|event| *event == TurnEvent::Idle);
        assert!(idle.is_some_and(|idle| idle < output), "{events:?}");
    }

    #[test]
    fn a_shown_pump_passes_stderr_lines_to_the_sink() {
        let sink = Arc::new(Recording::default());
        let (readers, _feed) = feed(&[], &["oops"]);
        let mut pump = Pump::start(&config(&sink), Display::Shown);
        pump.step(&readers, false).unwrap();
        assert_eq!(
            sink.events(),
            vec![
                TurnEvent::Busy(Wait::Response),
                TurnEvent::Stderr("oops\n".to_string())
            ]
        );
    }

    #[test]
    fn pump_event_counts_tools_and_finishes_without_readers() {
        use ralph_core::stream::{
            AssistantEvent, AssistantMessage, ResultEvent, ToolResult, UserEvent, UserMessage,
        };
        let assistant = |content| {
            StreamEvent::Assistant(AssistantEvent {
                message: AssistantMessage {
                    id: None,
                    content: vec![content],
                    model: None,
                    stop_reason: None,
                    usage: None,
                },
            })
        };
        let sink = Arc::new(Recording::default());
        let mut pump = Pump::start(&config(&sink), Display::Shown);
        pump.event(assistant(ContentBlock::ToolUse {
            id: "t1".into(),
            name: "Read".into(),
            input: serde_json::json!({}),
        }));
        assert_eq!(pump.pending_tool_count, 1);
        assert!(pump.request_at.is_some());
        pump.event(StreamEvent::User(UserEvent {
            message: UserMessage {
                id: None,
                content: vec![ToolResult {
                    result_type: Some("tool_result".into()),
                    tool_use_id: Some("t1".into()),
                    content: Some("ok".into()),
                    is_error: false,
                }],
            },
        }));
        assert_eq!(pump.pending_tool_count, 0);
        pump.event(assistant(ContentBlock::Text {
            text: "done\n".into(),
        }));
        pump.event(StreamEvent::Result(ResultEvent {
            subtype: Some("success".into()),
            total_cost_usd: None,
            cost_usd: None,
            duration_ms: None,
            duration_api_ms: None,
            usage: None,
            session_id: None,
            num_turns: None,
            result: None,
            terminal_reason: Some("end_turn".into()),
            model_usage: Default::default(),
        }));
        let result = pump.finish_events(0);
        assert_eq!(result.exit_code, 0);
        assert!(result.stderr.is_empty());
        assert_eq!(
            result.stream_result.costs.terminal_reason.as_deref(),
            Some("end_turn")
        );
        assert!(sink
            .events()
            .iter()
            .any(|event| matches!(event, TurnEvent::Blocks(_))));
    }

    #[test]
    fn a_silent_pump_emits_nothing() {
        let sink = Arc::new(Recording::default());
        let (readers, _feed) = feed(&[HELLO], &["oops"]);
        let mut pump = Pump::start(&config(&sink), Display::Silent);
        pump.step(&readers, false).unwrap();
        pump.finish(&readers, 0, true);
        assert_eq!(sink.events(), Vec::new());
    }

    #[test]
    fn the_pump_shows_tools_as_the_client_says() {
        let sink = Arc::new(Recording::default());
        [true, false].into_iter().for_each(|show_tools| {
            let pump = Pump::start(
                &SpinnerSubprocessConfig {
                    show_tools,
                    ..config(&sink)
                },
                Display::Silent,
            );
            assert_eq!(pump.collector.is_showing_tool_invocations(), show_tools);
        });
    }

    #[test]
    fn dropping_a_waiting_pump_stops_the_spinner() {
        let sink = Arc::new(Recording::default());
        drop(Pump::start(&config(&sink), Display::Shown));
        assert_eq!(
            sink.events(),
            vec![TurnEvent::Busy(Wait::Response), TurnEvent::Idle]
        );
    }

    fn failed(kind: io::ErrorKind) -> PromptWrite {
        Ok(Err(io::Error::from(kind)))
    }

    #[test]
    fn complete_write_has_no_warning() {
        assert_eq!(prompt_write_warning(&Ok(Ok(()))), None);
    }

    #[test]
    fn broken_pipe_warns_of_a_short_write() {
        let message = prompt_write_warning(&failed(io::ErrorKind::BrokenPipe)).unwrap();
        assert!(message.contains("short write"), "{message}");
        assert!(message.contains("broken pipe"), "{message}");
    }

    #[test]
    fn other_io_error_warns_with_its_text() {
        let message = prompt_write_warning(&failed(io::ErrorKind::PermissionDenied)).unwrap();
        assert!(message.contains("permission denied"), "{message}");
    }

    #[test]
    fn writer_panic_warns_without_panicking() {
        let panicked: PromptWrite = thread::spawn(|| -> io::Result<()> { panic!("boom") }).join();
        let message = prompt_write_warning(&panicked).unwrap();
        assert!(message.contains("panicked"), "{message}");
    }
}
