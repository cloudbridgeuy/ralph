use crate::chat_status::Status;
use crate::status_mode::{mode, Mode};
use crate::status_queue::queue_variants;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

mod build;
#[cfg(test)]
mod fixtures;
mod layout;

pub use build::{segments, store_label};
pub use layout::{bar, compose, layout};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Mode(Mode),
    Queue(Mode),
    Mail(Mode),
    Store,
    Workers,
    Watches,
    Thread(Mode),
    Cost(Mode),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shade {
    ModeIdleFg,
    ModeIdleBg,
    ModeNapFg,
    ModeNapBg,
    ModeInboxFg,
    ModeInboxBg,
    ModeWorkFg,
    ModeWorkBg,
    StoreFg,
    WorkersFg,
    WatchesFg,
    BaseBg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Separator {
    Block,
    Powerline,
    Bar,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub fg: Shade,
    pub bg: Shade,
    pub bold: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub variants: Vec<String>,
    pub priority: u8,
    pub side: Side,
    pub kind: Kind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed {
    pub segment: usize,
    pub variant: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Context {
    pub store: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Edge {
    Fill,
    Seg(Kind),
}

fn mode_bg(mode: Mode) -> Shade {
    match mode {
        Mode::Idle => Shade::ModeIdleBg,
        Mode::Nap => Shade::ModeNapBg,
        Mode::Inbox => Shade::ModeInboxBg,
        Mode::Work => Shade::ModeWorkBg,
    }
}

fn mode_fg(mode: Mode) -> Shade {
    match mode {
        Mode::Idle => Shade::ModeIdleFg,
        Mode::Nap => Shade::ModeNapFg,
        Mode::Inbox => Shade::ModeInboxFg,
        Mode::Work => Shade::ModeWorkFg,
    }
}

impl Kind {
    pub(super) fn bg(self) -> Shade {
        match self {
            Kind::Mode(mode)
            | Kind::Queue(mode)
            | Kind::Mail(mode)
            | Kind::Thread(mode)
            | Kind::Cost(mode) => mode_bg(mode),
            Kind::Store | Kind::Workers | Kind::Watches => Shade::BaseBg,
        }
    }

    pub(super) fn fg(self) -> Shade {
        match self {
            Kind::Mode(mode)
            | Kind::Queue(mode)
            | Kind::Mail(mode)
            | Kind::Thread(mode)
            | Kind::Cost(mode) => mode_fg(mode),
            Kind::Store => Shade::StoreFg,
            Kind::Workers => Shade::WorkersFg,
            Kind::Watches => Shade::WatchesFg,
        }
    }

    pub(super) fn bold(self) -> bool {
        matches!(self, Kind::Mode(_) | Kind::Cost(_))
    }
}

impl Edge {
    pub(super) fn bg(self) -> Shade {
        match self {
            Edge::Fill => Shade::BaseBg,
            Edge::Seg(kind) => kind.bg(),
        }
    }
}

impl Segment {
    pub(super) fn new(side: Side, kind: Kind, priority: u8, variants: Vec<String>) -> Self {
        Self {
            variants,
            priority,
            side,
            kind,
        }
    }
}
