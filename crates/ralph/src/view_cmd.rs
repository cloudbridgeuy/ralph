use crate::chat::terminal::{poll_input, read_input, Input, PartialMode, POLL_MS};
use crate::chat::{agent_style, view, Display};
use crate::cli::ViewCommandArgs;
use crate::highlight::ThemeConfig;
use crate::render::image::{detect, enable_images};
use crate::render::{cap_wrap_width, RenderContext};
use crate::signal;
use ralph_core::cancel::CancelToken;
use ralph_core::chat::resolve_wrap_width;
use ralph_core::chat_view::{quits, resolve_view_pager, show_plan, PagerCommand, Show, ViewPager};
use ralph_core::program_status::Stdout;
use ralph_server::config::{set_store_layer, AppConfig};
use ralph_server::disk_files::resolve;
use ralph_server::warn::{warn, warn_if_err};
use std::io::{self, ErrorKind, IsTerminal, Write};
use std::process::{Command, ExitCode, Stdio};
use std::thread;
use std::time::Duration;

pub fn run(args: ViewCommandArgs) -> ExitCode {
    warn_if_err(signal::init(), "Failed to initialize signal handler");
    match open(&args).and_then(|(text, pager)| show(&text, &pager)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("Error: {message}");
            if io::stdout().is_terminal() {
                wait_for_quit();
            }
            ExitCode::FAILURE
        }
    }
}

fn open(args: &ViewCommandArgs) -> Result<(String, ViewPager), String> {
    if let Some(store) = &args.store {
        set_store_layer(store);
    }
    let config = AppConfig::load().map_err(|error| error.to_string())?;
    let wrap_cap = resolve_wrap_width(None, None, config.chat.wrap_width)
        .map_err(|error| error.to_string())?;
    cap_wrap_width(wrap_cap);
    let theme_config = ThemeConfig::from_config_and_env();
    crate::warn::set_warning_color(theme_config.ui().warning);
    let (pager, pager_warning) = resolve_view_pager(config.chat.view_pager.as_deref());
    pager_warning.iter().for_each(warn);
    let color = io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    enable_images(detect(color));
    let viewed = view::load(&resolve(&args.file))?;
    let (highlighter, markdown) = agent_style(&theme_config, &config.theme);
    let display = Display {
        ctx: RenderContext::new(&highlighter, color)
            .with_diff(*theme_config.diff())
            .with_tools(*theme_config.tools()),
        theme: &markdown,
        color,
        ui: *theme_config.ui(),
    };
    Ok((view::render(&display, &viewed.text), pager))
}

fn show(text: &str, pager: &ViewPager) -> Result<(), String> {
    let stdout = if io::stdout().is_terminal() {
        Stdout::Tty
    } else {
        Stdout::NotTty
    };
    match show_plan(pager, io::stdin().is_terminal(), stdout) {
        Show::Print => print(text),
        Show::PrintAndWait => {
            print(text)?;
            wait_for_quit();
            Ok(())
        }
        Show::Page(command) => page(command, text),
    }
}

fn print(text: &str) -> Result<(), String> {
    let mut out = io::stdout();
    tolerate_closed_pipe(out.write_all(text.as_bytes()).and_then(|()| out.flush()))
}

fn tolerate_closed_pipe(written: io::Result<()>) -> Result<(), String> {
    match written {
        Err(error) if error.kind() != ErrorKind::BrokenPipe => {
            Err(format!("could not write the file: {error}"))
        }
        _ => Ok(()),
    }
}

fn page(pager: &PagerCommand, text: &str) -> Result<(), String> {
    let program = &pager.program;
    let spawned = Command::new(program)
        .args(&pager.args)
        .stdin(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => {
            print(text)?;
            warn(format!("could not run the pager {program}: {error}"));
            wait_for_quit();
            return Ok(());
        }
    };
    let written = child
        .stdin
        .take()
        .map_or(Ok(()), |mut input| input.write_all(text.as_bytes()));
    let status = child
        .wait()
        .map_err(|error| format!("could not wait for the pager {program}: {error}"))?;
    if !status.success() {
        return Err(format!("the pager {program} failed: {status}"));
    }
    tolerate_closed_pipe(written)
}

fn wait_for_quit() {
    if !io::stdin().is_terminal() {
        return;
    }
    let _terminal = PartialMode::enter();
    let cancel = CancelToken::default();
    let _armed = signal::arm(&cancel);
    while !signal::is_terminated() && !cancel.is_cancelled() {
        match poll_input(POLL_MS) {
            Input::Closed => return,
            Input::Idle => {}
            Input::Ready => {
                let key = read_input();
                if quits(&key) {
                    return;
                }
                if key.is_empty() {
                    thread::sleep(Duration::from_millis(POLL_MS as u64));
                }
            }
        }
    }
}
