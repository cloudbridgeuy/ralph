use crate::worker::Report;
use serde::{Deserialize, Serialize};

pub const PANE_FORMAT: &str = "#{pane_active} #{window_active} #{session_id}";
pub const CLIENT_FORMAT: &str = "#{client_flags}";
pub const NOTIFICATION_TITLE: &str = "ralph chat";
const FOCUSED_FLAG: &str = "focused";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Visible,
    Hidden,
    Unknown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UnknownFocus {
    #[default]
    Alert,
    Silent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlertConfig {
    pub sound: bool,
    pub notification: bool,
    pub unknown_focus: UnknownFocus,
}

impl AlertConfig {
    pub fn enabled(&self) -> bool {
        self.sound || self.notification
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    pub sound: bool,
    pub notification: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneState {
    pub visible: bool,
    pub session: String,
}

pub fn fresh_reports<'a>(known: &[Report], pending: &'a [Report]) -> Vec<&'a Report> {
    pending
        .iter()
        .filter(|report| !known.contains(report))
        .collect()
}

pub fn parse_pane(line: &str) -> Option<PaneState> {
    let mut fields = line.split_whitespace();
    let pane_active = fields.next()?;
    let window_active = fields.next()?;
    let session = fields.next()?;
    Some(PaneState {
        visible: pane_active == "1" && window_active == "1",
        session: session.to_string(),
    })
}

pub fn tmux_focus(pane: Option<&PaneState>, clients: Option<&str>) -> Focus {
    let (Some(pane), Some(clients)) = (pane, clients) else {
        return Focus::Unknown;
    };
    let flags: Vec<&str> = clients.lines().filter(|line| !line.is_empty()).collect();
    if flags.is_empty() {
        return Focus::Unknown;
    }
    let focused = flags
        .iter()
        .any(|line| line.split(',').any(|flag| flag == FOCUSED_FLAG));
    if pane.visible && focused {
        Focus::Visible
    } else {
        Focus::Hidden
    }
}

pub fn inbox_alert(fresh: &[&Report], focus: Focus, config: &AlertConfig) -> Option<Alert> {
    let alert = match focus {
        Focus::Visible => false,
        Focus::Hidden => true,
        Focus::Unknown => config.unknown_focus == UnknownFocus::Alert,
    };
    (alert && !fresh.is_empty() && config.enabled()).then(|| Alert {
        sound: config.sound,
        notification: config.notification.then(|| notification_body(fresh)),
    })
}

fn notification_body(fresh: &[&Report]) -> String {
    fresh
        .iter()
        .map(|report| format!("{} {}", report.worker, report.status))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn osc9(body: &str, in_tmux: bool) -> String {
    let clean: String = body.chars().filter(|c| !c.is_control()).collect();
    let osc = format!("\x1b]9;{clean}\x07");
    if in_tmux {
        format!("\x1bPtmux;{}\x1b\\", osc.replace('\x1b', "\x1b\x1b"))
    } else {
        osc
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(worker: &str, status: &str) -> Report {
        Report {
            worker: worker.into(),
            at: "2026-10-12T00:00:00Z".into(),
            status: status.into(),
            text: "summary".into(),
        }
    }

    fn config(unknown_focus: UnknownFocus) -> AlertConfig {
        AlertConfig {
            sound: true,
            notification: true,
            unknown_focus,
        }
    }

    fn visible_pane() -> PaneState {
        PaneState {
            visible: true,
            session: "$3".into(),
        }
    }

    #[test]
    fn fresh_reports_skips_known() {
        let known = vec![report("a", "done")];
        let pending = vec![report("a", "done"), report("b", "question")];
        assert_eq!(fresh_reports(&known, &pending), vec![&pending[1]]);
        assert!(fresh_reports(&pending, &pending).is_empty());
    }

    #[test]
    fn parse_pane_reads_flags_and_session() {
        assert_eq!(parse_pane("1 1 $3\n"), Some(visible_pane()));
        assert_eq!(
            parse_pane("0 1 $3"),
            Some(PaneState {
                visible: false,
                session: "$3".into()
            })
        );
        assert_eq!(parse_pane("1 0 $3").map(|pane| pane.visible), Some(false));
        assert_eq!(parse_pane("1 1"), None);
        assert_eq!(parse_pane(""), None);
    }

    #[test]
    fn tmux_focus_visible_needs_active_pane_window_and_focused_client() {
        let pane = visible_pane();
        assert_eq!(
            tmux_focus(Some(&pane), Some("attached,focused,UTF-8\n")),
            Focus::Visible
        );
        assert_eq!(
            tmux_focus(
                Some(&pane),
                Some("attached,UTF-8\nattached,focused,UTF-8\n")
            ),
            Focus::Visible
        );
        assert_eq!(
            tmux_focus(Some(&pane), Some("attached,UTF-8\n")),
            Focus::Hidden
        );
        let hidden = PaneState {
            visible: false,
            ..visible_pane()
        };
        assert_eq!(
            tmux_focus(Some(&hidden), Some("attached,focused,UTF-8\n")),
            Focus::Hidden
        );
    }

    #[test]
    fn tmux_focus_unknown_without_query_or_client() {
        let pane = visible_pane();
        assert_eq!(tmux_focus(None, Some("attached,focused")), Focus::Unknown);
        assert_eq!(tmux_focus(Some(&pane), None), Focus::Unknown);
        assert_eq!(tmux_focus(Some(&pane), Some("")), Focus::Unknown);
        assert_eq!(tmux_focus(None, None), Focus::Unknown);
    }

    #[test]
    fn focused_flag_matches_whole_word() {
        assert_eq!(
            tmux_focus(Some(&visible_pane()), Some("attached,unfocused")),
            Focus::Hidden
        );
    }

    #[test]
    fn visible_chat_gets_no_alert() {
        let reports = [report("a", "done")];
        let fresh: Vec<&Report> = reports.iter().collect();
        assert_eq!(
            inbox_alert(&fresh, Focus::Visible, &config(UnknownFocus::Alert)),
            None
        );
    }

    #[test]
    fn hidden_chat_gets_sound_and_notification() {
        let reports = [report("GUZ-422", "done"), report("GUZ-423", "question")];
        let fresh: Vec<&Report> = reports.iter().collect();
        assert_eq!(
            inbox_alert(&fresh, Focus::Hidden, &config(UnknownFocus::Silent)),
            Some(Alert {
                sound: true,
                notification: Some("GUZ-422 done, GUZ-423 question".into()),
            })
        );
    }

    #[test]
    fn unknown_focus_follows_setting() {
        let reports = [report("a", "blocked")];
        let fresh: Vec<&Report> = reports.iter().collect();
        assert!(inbox_alert(&fresh, Focus::Unknown, &config(UnknownFocus::Alert)).is_some());
        assert_eq!(
            inbox_alert(&fresh, Focus::Unknown, &config(UnknownFocus::Silent)),
            None
        );
    }

    #[test]
    fn config_turns_each_part_off() {
        let reports = [report("a", "done")];
        let fresh: Vec<&Report> = reports.iter().collect();
        let sound_only = AlertConfig {
            notification: false,
            ..config(UnknownFocus::Alert)
        };
        assert_eq!(
            inbox_alert(&fresh, Focus::Hidden, &sound_only),
            Some(Alert {
                sound: true,
                notification: None
            })
        );
        let notification_only = AlertConfig {
            sound: false,
            ..config(UnknownFocus::Alert)
        };
        assert_eq!(
            inbox_alert(&fresh, Focus::Hidden, &notification_only),
            Some(Alert {
                sound: false,
                notification: Some("a done".into())
            })
        );
        let off = AlertConfig {
            sound: false,
            notification: false,
            unknown_focus: UnknownFocus::Alert,
        };
        assert_eq!(inbox_alert(&fresh, Focus::Hidden, &off), None);
    }

    #[test]
    fn no_fresh_reports_no_alert() {
        assert_eq!(
            inbox_alert(&[], Focus::Hidden, &config(UnknownFocus::Alert)),
            None
        );
    }

    #[test]
    fn osc9_wraps_for_tmux_and_strips_controls() {
        assert_eq!(osc9("a\x07 done", false), "\x1b]9;a done\x07");
        assert_eq!(
            osc9("a done", true),
            "\x1bPtmux;\x1b\x1b]9;a done\x07\x1b\\"
        );
    }
}
