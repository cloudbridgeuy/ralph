use crate::ansi::{DIM, RESET};
use crate::signal;
use crate::subprocess::kill_process_group;
use crate::warn::warn;
use ralph_core::chat_cell::{
    build_message, diff_snapshots, display_results, parse_cells, parse_snapshots, shell_stdin,
    strip_shell_preamble, Cell, CellLimits, CellRun, SavedState, ShellKind,
};
use std::ffi::OsString;
use std::io::{self, IsTerminal, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use crate::chat::TurnOutcome;

const POLL: Duration = Duration::from_millis(50);
const SHELL_RUNTIME: &str = "shell";
const SNAPSHOT_FD: i32 = 3;

#[derive(Default)]
pub struct ShellState {
    saved: SavedState,
    warned_unsupported: bool,
}

pub struct Prepared {
    pub sent: String,
    pub shown: Option<String>,
}

enum Target {
    Plain(PathBuf),
    Shell {
        program: PathBuf,
        kind: Option<ShellKind>,
    },
}

struct Launch<'a> {
    program: &'a Path,
    args: &'static [&'static str],
    stdin: String,
    snapshot: bool,
}

struct Finished {
    run: CellRun,
    snapshot: Vec<u8>,
}

fn is_executable(path: &Path) -> bool {
    path.metadata()
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

fn find_on_path(runtime: &str, path: &OsString) -> Option<PathBuf> {
    std::env::split_paths(path)
        .map(|dir| dir.join(runtime))
        .find(|candidate| is_executable(candidate))
}

fn resolve_shell() -> Option<Target> {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|shell| !shell.is_empty())
        .unwrap_or_else(|| "/bin/sh".to_string());
    is_executable(Path::new(&shell)).then(|| Target::Shell {
        kind: ShellKind::detect(&shell),
        program: PathBuf::from(shell),
    })
}

fn resolve_runtimes(cells: &[Cell]) -> Result<Vec<Target>, Vec<&str>> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let found: Vec<(&str, Option<Target>)> = cells
        .iter()
        .map(|cell| {
            let target = if cell.runtime == SHELL_RUNTIME {
                resolve_shell()
            } else {
                find_on_path(&cell.runtime, &path).map(Target::Plain)
            };
            (cell.runtime.as_str(), target)
        })
        .collect();
    let mut missing: Vec<&str> = found
        .iter()
        .filter(|(_, target)| target.is_none())
        .map(|(runtime, _)| *runtime)
        .collect();
    missing.dedup();
    if missing.is_empty() {
        Ok(found.into_iter().filter_map(|(_, target)| target).collect())
    } else {
        Err(missing)
    }
}

fn capture(mut source: impl Read, captured: Arc<Mutex<Vec<u8>>>) {
    let mut chunk = [0u8; 4096];
    while let Ok(read @ 1..) = source.read(&mut chunk) {
        captured
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(&chunk[..read]);
    }
}

fn spawn_capture(
    pipe: impl Read + Send + 'static,
    captured: &Arc<Mutex<Vec<u8>>>,
) -> thread::JoinHandle<()> {
    let captured = Arc::clone(captured);
    thread::spawn(move || capture(pipe, captured))
}

fn detach_from_terminal(command: &mut Command) {
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

fn route_to_snapshot_fd(command: &mut Command, write_fd: i32) {
    unsafe {
        command.pre_exec(move || {
            if write_fd == SNAPSHOT_FD {
                let flags = libc::fcntl(SNAPSHOT_FD, libc::F_GETFD);
                if flags < 0
                    || libc::fcntl(SNAPSHOT_FD, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0
                {
                    return Err(io::Error::last_os_error());
                }
            } else if libc::dup2(write_fd, SNAPSHOT_FD) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

fn run_cell(launch: &Launch, cell: &Cell) -> io::Result<Option<Finished>> {
    signal::interrupt_flag().store(false, Ordering::SeqCst);
    let mut command = Command::new(launch.program);
    let (output_reader, output_writer) = io::pipe()?;
    command
        .args(launch.args)
        .stdin(Stdio::piped())
        .stdout(output_writer.try_clone()?)
        .stderr(output_writer);
    detach_from_terminal(&mut command);
    let snapshot_pipe = if launch.snapshot {
        let (reader, writer) = io::pipe()?;
        route_to_snapshot_fd(&mut command, writer.as_raw_fd());
        Some((reader, writer))
    } else {
        None
    };
    let mut child = command.spawn()?;
    drop(command);
    let snapshot_reader = snapshot_pipe.map(|(reader, writer)| {
        drop(writer);
        reader
    });
    let captured = Arc::new(Mutex::new(Vec::new()));
    let snapshot = Arc::new(Mutex::new(Vec::new()));
    let stdin = child.stdin.take();
    let input = launch.stdin.clone();
    let writer = thread::spawn(move || {
        if let Some(mut stdin) = stdin {
            let _ = stdin.write_all(input.as_bytes());
        }
    });
    let readers = [
        Some(spawn_capture(output_reader, &captured)),
        snapshot_reader.map(|reader| spawn_capture(reader, &snapshot)),
    ];
    let status = loop {
        if signal::is_interrupted() {
            kill_process_group(&mut child);
            break None;
        }
        match child.try_wait()? {
            Some(status) => break Some(status),
            None => thread::sleep(POLL),
        }
    };
    let _ = writer.join();
    readers.into_iter().flatten().for_each(|reader| {
        let _ = reader.join();
    });
    signal::interrupt_flag().store(false, Ordering::SeqCst);
    let output = captured.lock().unwrap_or_else(PoisonError::into_inner);
    let snapshot = snapshot.lock().unwrap_or_else(PoisonError::into_inner);
    Ok(status.map(|status| Finished {
        run: CellRun {
            cell: cell.clone(),
            exit: status
                .code()
                .or_else(|| status.signal().map(|sig| 128 + sig)),
            output: shown_output(&output, launch.snapshot),
        },
        snapshot: snapshot.clone(),
    }))
}

fn shown_output(raw: &[u8], stateful: bool) -> String {
    let text = String::from_utf8_lossy(raw);
    if stateful {
        strip_shell_preamble(&text).to_string()
    } else {
        text.into_owned()
    }
}

fn plain_stdin(cell: &Cell) -> String {
    format!("{}\n", cell.code)
}

fn launch_for<'a>(target: &'a Target, cell: &Cell, state: &mut ShellState) -> Launch<'a> {
    match target {
        Target::Plain(program) => Launch {
            program,
            args: &[],
            stdin: plain_stdin(cell),
            snapshot: false,
        },
        Target::Shell {
            program,
            kind: Some(kind),
        } => Launch {
            program,
            args: &["-i"],
            stdin: shell_stdin(*kind, &state.saved, &cell.code),
            snapshot: true,
        },
        Target::Shell {
            program,
            kind: None,
        } => {
            if !state.warned_unsupported {
                state.warned_unsupported = true;
                warn("`!shell` keeps state only for zsh and bash; this shell runs without it.");
            }
            Launch {
                program,
                args: &["-i"],
                stdin: plain_stdin(cell),
                snapshot: false,
            }
        }
    }
}

fn absorb_snapshot(state: &mut ShellState, snapshot: &[u8]) {
    state.saved = match parse_snapshots(snapshot) {
        Some((start, end)) => diff_snapshots(&end, &start),
        None => {
            warn("`!shell` state could not be read; starting clean.");
            SavedState::default()
        }
    };
}

fn announce(index: usize, runtime: &str) {
    let color = io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let (dim, reset) = if color { (DIM, RESET) } else { ("", "") };
    println!("{dim}▸ cell {index} · {runtime}{reset}");
}

pub fn prepare(
    message: &str,
    limits: CellLimits,
    state: &mut ShellState,
) -> Result<Prepared, TurnOutcome> {
    let cells = parse_cells(message);
    if cells.is_empty() {
        return Ok(Prepared {
            sent: message.to_string(),
            shown: None,
        });
    }
    let targets = resolve_runtimes(&cells).map_err(|missing| {
        missing
            .iter()
            .for_each(|runtime| warn(format!("runtime `{runtime}` not found")));
        TurnOutcome::Unanswered
    })?;
    let mut runs = Vec::new();
    for (index, (target, cell)) in targets.iter().zip(&cells).enumerate() {
        announce(index + 1, &cell.runtime);
        let launch = launch_for(target, cell, state);
        match run_cell(&launch, cell) {
            Ok(Some(finished)) => {
                if launch.snapshot {
                    absorb_snapshot(state, &finished.snapshot);
                }
                runs.push(finished.run);
            }
            Ok(None) => {
                warn("Cell cancelled.");
                return Err(if signal::is_terminated() {
                    TurnOutcome::Exit
                } else {
                    TurnOutcome::Unanswered
                });
            }
            Err(e) => {
                warn(format!("cell `{}` failed to run: {e}", cell.runtime));
                if launch.snapshot {
                    state.saved = SavedState::default();
                }
                return Err(TurnOutcome::Unanswered);
            }
        }
    }
    Ok(Prepared {
        sent: build_message(message, &runs, limits),
        shown: Some(display_results(message, &runs)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_lookup_needs_an_executable_file_by_exact_name() {
        let dir = tempfile::tempdir().unwrap();
        let tool = dir.path().join("tool");
        std::fs::write(&tool, "").unwrap();
        let path = std::env::join_paths([dir.path()]).unwrap();
        assert_eq!(find_on_path("tool", &path), None);
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(find_on_path("tool", &path), Some(tool));
        assert_eq!(find_on_path("too", &path), None);
    }
}
