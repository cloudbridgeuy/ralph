use crate::subprocess::spawn_detached;
use crate::warn::warn_if_err;
use ralph_core::inbox_alert::{
    osc9, parse_pane, tmux_focus, Alert, Focus, CLIENT_FORMAT, NOTIFICATION_TITLE, PANE_FORMAT,
};
use std::io::{self, Write};
use std::process::{Command, Stdio};

const TMUX_PANE_ENV: &str = "TMUX_PANE";
const SOUND_FILE: &str = "/System/Library/Sounds/Glass.aiff";
const BELL: &str = "\x07";

fn tmux_output(args: &[&str]) -> Option<String> {
    let output = Command::new("tmux")
        .args(args)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn focus() -> Focus {
    let Some(pane_id) = std::env::var_os(TMUX_PANE_ENV) else {
        return Focus::Unknown;
    };
    let pane_id = pane_id.to_string_lossy();
    let pane = tmux_output(&["display", "-p", "-t", &pane_id, PANE_FORMAT])
        .and_then(|line| parse_pane(&line));
    let clients = pane
        .as_ref()
        .and_then(|pane| tmux_output(&["list-clients", "-t", &pane.session, "-F", CLIENT_FORMAT]));
    tmux_focus(pane.as_ref(), clients.as_deref())
}

fn write_terminal(text: &str) -> io::Result<()> {
    let mut stdout = io::stdout();
    stdout.write_all(text.as_bytes())?;
    stdout.flush()
}

fn play_sound() -> io::Result<()> {
    if cfg!(target_os = "macos") {
        spawn_detached(Command::new("afplay").arg(SOUND_FILE))
    } else {
        write_terminal(BELL)
    }
}

fn notify(body: &str) -> io::Result<()> {
    if cfg!(target_os = "macos") {
        spawn_detached(Command::new("osascript").args([
            "-e",
            "on run argv",
            "-e",
            "display notification (item 2 of argv) with title (item 1 of argv)",
            "-e",
            "end run",
            NOTIFICATION_TITLE,
            body,
        ]))
    } else {
        write_terminal(&osc9(body, std::env::var_os(TMUX_PANE_ENV).is_some()))
    }
}

pub fn deliver(alert: &Alert) {
    if alert.sound {
        warn_if_err(play_sound(), "Failed to play inbox sound");
    }
    if let Some(body) = &alert.notification {
        warn_if_err(notify(body), "Failed to send inbox notification");
    }
}
