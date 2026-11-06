use super::views::View;
use super::window::Window;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CostsArgs {
    pub view: View,
    pub window: Window,
}

fn parse_view(word: &str) -> Option<View> {
    match word {
        "harness" => Some(View::Harness),
        "model" => Some(View::HarnessModel),
        "role" => Some(View::Role),
        "ticket" => Some(View::Ticket),
        _ => None,
    }
}

fn parse_window(word: &str) -> Option<Window> {
    match word {
        "today" => Some(Window::Today),
        "7d" => Some(Window::Week),
        "all" => Some(Window::All),
        _ => None,
    }
}

pub fn parse_costs_args(words: &[&str]) -> Option<CostsArgs> {
    let (view, rest) = match words.split_first() {
        Some((first, rest)) => parse_view(first).map_or((View::default(), words), |v| (v, rest)),
        None => (View::default(), words),
    };
    let window = match rest {
        [] => Window::default(),
        [word] => parse_window(word)?,
        _ => return None,
    };
    Some(CostsArgs { view, window })
}

#[cfg(test)]
mod tests;
