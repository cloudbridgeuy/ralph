use crate::chat::monitor::Monitor;
use crate::chat::prompt::screen_columns;
use crate::chat::terminal::{poll_input, read_input, Input, PartialMode, POLL_MS};
use crate::config::ChatSection;
use crate::inbox_alert::deliver;
use crate::signal;
use ralph_core::chat_send_it::{
    blocked_alert, blocked_report, cancel_notice, closed_notice, countdown_frame, decide,
    no_terminal_notice, remaining_seconds, status_text, tracks_worker, worker_closed,
    worker_flagged, Decision, Facts, Limits, Run, SendIt, Step, CLEAR_LINE,
};
use ralph_core::chat_status::Status;
use ralph_core::worker::Report;
use std::io::{self, IsTerminal, Write};
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

struct Armed {
    run: Run,
    tracked: bool,
}

pub(super) struct Offer<'a> {
    pub text: &'a str,
    pub step: Option<Step>,
}

pub(super) struct Auto {
    pub step: Step,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Countdown {
    Elapsed,
    Cancelled,
}

pub(super) struct Shell {
    armed: Option<Armed>,
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
    pub(super) fn new(config: &ChatSection, color: bool) -> Self {
        Self {
            armed: None,
            limits: Limits {
                max_turns: config.send_it_max_turns,
                max_usd: config.send_it_max_usd,
            },
            seconds: config.send_it_seconds,
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

    pub(super) fn command(&mut self, command: SendIt, typed_at_prompt: bool, status: &Status) {
        match command {
            SendIt::Arm(_) if !typed_at_prompt => {
                println!("/send-it can only be armed from a line typed at the prompt.");
            }
            SendIt::Arm(ticket) => {
                self.armed = Some(Armed {
                    tracked: tracks_worker(&ticket, status),
                    run: Run::new(&ticket),
                });
                self.print_status();
            }
            SendIt::Off => {
                self.armed = None;
                self.print_status();
            }
            SendIt::Status => self.print_status(),
        }
    }

    fn print_status(&self) {
        let run = self.armed.as_ref().map(|armed| &armed.run);
        println!("{}", status_text(run, &self.limits, self.seconds));
    }

    fn halt(&mut self, notice: &str, monitor: &Monitor) {
        self.armed = None;
        println!("{notice}");
        if let Some(alert) = blocked_alert(notice, &monitor.alert) {
            deliver(&alert);
        }
        monitor.term.send(&blocked_report(notice));
    }

    pub(super) fn at_turn_end(
        &mut self,
        offer: Option<&Offer>,
        schedule_pending: bool,
        monitor: &Monitor,
    ) -> Option<Auto> {
        let armed = self.armed.as_ref()?;
        if worker_closed(armed.tracked, &armed.run.ticket, &monitor.status()) {
            println!("{}", closed_notice(&armed.run));
            self.armed = None;
            return None;
        }
        let offer = offer?;
        let run = armed.run.clone();
        let facts = Facts {
            text: offer.text,
            step: offer.step,
            worker_flagged: monitor
                .inbox
                .peek()
                .map_or(true, |reports| worker_flagged(&reports, &self.drained)),
            schedule_pending,
        };
        let notice = match decide(&run, &self.limits, &facts) {
            Decision::Countdown => {
                let step = offer.step?;
                match countdown(offer.text, self.seconds, self.color) {
                    Some(Countdown::Elapsed) => {
                        return Some(Auto {
                            step,
                            text: offer.text.to_string(),
                        });
                    }
                    Some(Countdown::Cancelled) => {
                        println!("{}", cancel_notice(&run));
                        self.armed = None;
                        return None;
                    }
                    None => no_terminal_notice(&run),
                }
            }
            Decision::Manual(reason) => reason.notice(&run),
            Decision::Stop(reason) => reason.notice(&run),
        };
        self.halt(&notice, monitor);
        None
    }

    pub(super) fn record(&mut self, auto: &Auto, cost_usd: Option<f64>) {
        if let Some(armed) = &mut self.armed {
            armed.run.record(auto.step, &auto.text, cost_usd);
        }
    }
}
