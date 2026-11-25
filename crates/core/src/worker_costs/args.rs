use super::views::View;
use super::window::Window;
use crate::chat_spec::Keyword;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CostsArgs {
    pub view: View,
    pub window: Window,
}

impl Keyword for View {
    const ALL: &'static [Self] = &[View::HarnessModel, View::Harness, View::Role, View::Ticket];

    fn word(self) -> &'static str {
        match self {
            View::HarnessModel => "model",
            View::Harness => "harness",
            View::Role => "role",
            View::Ticket => "ticket",
        }
    }
}

impl Keyword for Window {
    const ALL: &'static [Self] = &[Window::Today, Window::Week, Window::All];

    fn word(self) -> &'static str {
        match self {
            Window::Today => "today",
            Window::Week => "7d",
            Window::All => "all",
        }
    }
}

pub fn parse_costs_args(words: &[&str]) -> Option<CostsArgs> {
    let (view, rest) = match words.split_first() {
        Some((first, rest)) => View::parse(first).map_or((View::default(), words), |v| (v, rest)),
        None => (View::default(), words),
    };
    let window = match rest {
        [] => Window::default(),
        [word] => Window::parse(word)?,
        _ => return None,
    };
    Some(CostsArgs { view, window })
}

#[cfg(test)]
mod tests;
