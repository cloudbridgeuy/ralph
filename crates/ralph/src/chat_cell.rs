use crate::signal;
use crate::subprocess::kill_process_group;
use crate::warn::warn;
use ralph_core::chat_cell::{build_message, parse_cells, Cell, CellLimits, CellRun};
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use crate::chat::TurnOutcome;

const POLL: Duration = Duration::from_millis(50);

fn find_on_path(runtime: &str, path: &OsString) -> Option<PathBuf> {
    std::env::split_paths(path)
        .map(|dir| dir.join(runtime))
        .find(|candidate| {
            candidate
                .metadata()
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
}

fn resolve_runtimes(cells: &[Cell]) -> Result<Vec<PathBuf>, Vec<&str>> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let found: Vec<(&str, Option<PathBuf>)> = cells
        .iter()
        .map(|cell| (cell.runtime.as_str(), find_on_path(&cell.runtime, &path)))
        .collect();
    let mut missing: Vec<&str> = found
        .iter()
        .filter(|(_, path)| path.is_none())
        .map(|(runtime, _)| *runtime)
        .collect();
    missing.dedup();
    if missing.is_empty() {
        Ok(found.into_iter().filter_map(|(_, path)| path).collect())
    } else {
        Err(missing)
    }
}

fn tee(mut source: impl Read, captured: Arc<Mutex<Vec<u8>>>) {
    let mut chunk = [0u8; 4096];
    while let Ok(read @ 1..) = source.read(&mut chunk) {
        let mut out = io::stdout().lock();
        let _ = out.write_all(&chunk[..read]);
        let _ = out.flush();
        captured
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(&chunk[..read]);
    }
}

fn spawn_tee(
    pipe: Option<impl Read + Send + 'static>,
    captured: &Arc<Mutex<Vec<u8>>>,
) -> Option<thread::JoinHandle<()>> {
    let captured = Arc::clone(captured);
    pipe.map(|pipe| thread::spawn(move || tee(pipe, captured)))
}

fn run_cell(program: &PathBuf, cell: &Cell) -> io::Result<Option<CellRun>> {
    signal::interrupt_flag().store(false, Ordering::SeqCst);
    let mut child = Command::new(program)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()?;
    let captured = Arc::new(Mutex::new(Vec::new()));
    let stdin = child.stdin.take();
    let code = cell.code.clone();
    let writer = thread::spawn(move || {
        if let Some(mut stdin) = stdin {
            let _ = stdin.write_all(code.as_bytes());
            let _ = stdin.write_all(b"\n");
        }
    });
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let readers = [spawn_tee(stdout, &captured), spawn_tee(stderr, &captured)];
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
    Ok(status.map(|status| CellRun {
        cell: cell.clone(),
        exit: status
            .code()
            .or_else(|| status.signal().map(|sig| 128 + sig)),
        output: String::from_utf8_lossy(&output).into_owned(),
    }))
}

pub fn prepare(message: &str, limits: CellLimits) -> Result<String, TurnOutcome> {
    let cells = parse_cells(message);
    if cells.is_empty() {
        return Ok(message.to_string());
    }
    let programs = resolve_runtimes(&cells).map_err(|missing| {
        missing
            .iter()
            .for_each(|runtime| warn(format!("runtime `{runtime}` not found")));
        TurnOutcome::Unanswered
    })?;
    let mut runs = Vec::new();
    for (program, cell) in programs.iter().zip(&cells) {
        match run_cell(program, cell) {
            Ok(Some(run)) => {
                if run.exit != Some(0) {
                    println!("[exit {}]", run.exit.unwrap_or(-1));
                }
                runs.push(run);
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
                return Err(TurnOutcome::Unanswered);
            }
        }
    }
    Ok(build_message(message, &runs, limits))
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
