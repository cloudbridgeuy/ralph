use crate::chat::monitor::Monitor;
use crate::chat::prompt::screen_columns;
use crate::chat::terminal::{poll_input, read_input, Input, PartialMode, POLL_MS};
use crate::config::SendItSettings;
use crate::inbox_alert::deliver;
use crate::signal;
use ralph_core::chat_send_it::{
    blocked_alert, blocked_report, cancel_notice, countdown_frame, decide, no_terminal_notice,
    remaining_seconds, status_text, worker_flagged, Decision, Facts, Limits, Run, SendIt,
    CLEAR_LINE,
};
use ralph_core::worker::Report;
use std::io::{self, IsTerminal, Write};
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Countdown {
    Elapsed,
    Cancelled,
}

pub(super) struct Shell {
    run: Option<Run>,
    limits: Limits,
    seconds: u64,
    color: bool,
    drained: Vec<Report>,
}

fn countdown(text: &str, seconds: u64, color: bool) -> Option<Countdown> {
    if !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
        return None;
    }
    let _terminal = PartialMode::enter()?;
    let interrupt = signal::interrupt_flag();
    interrupt.store(false, Ordering::SeqCst);
    let started = Instant::now();
    let mut shown = None;
    let mut out = io::stdout();
    let result = loop {
        let left = remaining_seconds(
            u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            seconds,
        );
        if left == 0 {
            break Countdown::Elapsed;
        }
        if shown != Some(left) {
            let _ = write!(
                out,
                "{}",
                countdown_frame(left, text, screen_columns(), color)
            );
            let _ = out.flush();
            shown = Some(left);
        }
        if signal::is_terminated() || interrupt.swap(false, Ordering::SeqCst) {
            break Countdown::Cancelled;
        }
        match poll_input(POLL_MS) {
            Input::Idle => {}
            Input::Closed => break Countdown::Cancelled,
            Input::Ready => {
                if read_input().is_empty() {
                    thread::sleep(Duration::from_millis(POLL_MS as u64));
                } else {
                    break Countdown::Cancelled;
                }
            }
        }
    };
    let _ = write!(out, "{CLEAR_LINE}");
    let _ = out.flush();
    Some(result)
}

impl Shell {
    pub(super) fn new(settings: &SendItSettings, color: bool) -> Self {
        Self {
            run: None,
            limits: Limits {
                max_turns: settings.max_turns,
                max_usd: settings.max_usd,
            },
            seconds: settings.seconds,
            color,
            drained: Vec::new(),
        }
    }

    pub(super) fn begin_turn(&mut self) {
        self.drained.clear();
    }

    pub(super) fn note_drained(&mut self, reports: &[Report]) {
        self.drained.extend_from_slice(reports);
    }

    pub(super) fn command(&mut self, command: SendIt, typed_at_prompt: bool) {
        match command {
            SendIt::On if !typed_at_prompt => {
                println!("/send-it can only be turned on from a line typed at the prompt.");
                return;
            }
            SendIt::On => {
                self.run.get_or_insert_with(Run::default);
            }
            SendIt::Off => self.run = None,
        }
        println!(
            "{}",
            status_text(self.run.as_ref(), &self.limits, self.seconds)
        );
    }

    fn alert(notice: &str, monitor: &Monitor) {
        println!("{notice}");
        if let Some(alert) = blocked_alert(notice, &monitor.alert) {
            deliver(&alert);
        }
        monitor.term.send(&blocked_report(notice));
    }

    pub(super) fn at_turn_end(
        &mut self,
        offer: Option<&str>,
        schedule_pending: bool,
        monitor: &Monitor,
    ) -> Option<String> {
        let run = self.run.clone()?;
        let offer = offer?;
        let facts = Facts {
            text: offer,
            worker_flagged: monitor
                .inbox
                .peek()
                .map_or(true, |reports| worker_flagged(&reports, &self.drained)),
            schedule_pending,
        };
        let notice = match decide(&run, &self.limits, &facts) {
            Decision::Countdown => match countdown(offer, self.seconds, self.color) {
                Some(Countdown::Elapsed) => return Some(offer.to_string()),
                Some(Countdown::Cancelled) => {
                    println!("{}", cancel_notice());
                    self.run = None;
                    return None;
                }
                None => no_terminal_notice(),
            },
            Decision::Manual(reason) => {
                Self::alert(&reason.notice(), monitor);
                return None;
            }
            Decision::Stop(reason) => reason.notice(&run),
        };
        self.run = None;
        Self::alert(&notice, monitor);
        None
    }

    pub(super) fn record(&mut self, text: &str, cost_usd: Option<f64>) {
        if let Some(run) = &mut self.run {
            run.record(text, cost_usd);
        }
    }
}
