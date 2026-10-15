//! ANSI escape sequences for terminal control.
//!
//! This module provides common ANSI escape codes used throughout the application
//! for terminal manipulation and text styling.

/// Carriage return - moves cursor to beginning of line.
pub const CR: &str = "\r";

/// Clear from cursor to end of line.
pub const CLEAR_EOL: &str = "\x1b[K";

/// Set foreground color to cyan.
pub const CYAN: &str = "\x1b[36m";

/// Set foreground color to yellow.
pub const YELLOW: &str = "\x1b[33m";

pub const GREEN: &str = "\x1b[32m";

pub const RED: &str = "\x1b[31m";

pub const BLUE: &str = "\x1b[34m";

/// Set text to bold.
pub const BOLD: &str = "\x1b[1m";

/// Set text to dim.
pub const DIM: &str = "\x1b[2m";

/// Reset all text formatting.
pub const RESET: &str = "\x1b[0m";

fn color_code(color: termimad::crossterm::style::Color) -> String {
    use termimad::crossterm::style::Color;
    match color {
        Color::Reset => "39".to_string(),
        Color::Black => "30".to_string(),
        Color::DarkRed => "31".to_string(),
        Color::DarkGreen => "32".to_string(),
        Color::DarkYellow => "33".to_string(),
        Color::DarkBlue => "34".to_string(),
        Color::DarkMagenta => "35".to_string(),
        Color::DarkCyan => "36".to_string(),
        Color::Grey => "37".to_string(),
        Color::DarkGrey => "90".to_string(),
        Color::Red => "91".to_string(),
        Color::Green => "92".to_string(),
        Color::Yellow => "93".to_string(),
        Color::Blue => "94".to_string(),
        Color::Magenta => "95".to_string(),
        Color::Cyan => "96".to_string(),
        Color::White => "97".to_string(),
        Color::Rgb { r, g, b } => format!("38;2;{r};{g};{b}"),
        Color::AnsiValue(n) => format!("38;5;{n}"),
    }
}

pub fn fg(color: Option<termimad::crossterm::style::Color>) -> String {
    color.map_or_else(String::new, |color| format!("\x1b[{}m", color_code(color)))
}

pub fn bg(color: Option<termimad::crossterm::style::Color>) -> String {
    let Some(color) = color else {
        return String::new();
    };
    let code = color_code(color);
    let background = match code.strip_prefix("38;") {
        Some(rest) => format!("48;{rest}"),
        None => code
            .parse::<u8>()
            .map_or(code, |named| (named + 10).to_string()),
    };
    format!("\x1b[{background}m")
}

#[cfg(test)]
mod tests {
    use super::*;
    use termimad::crossterm::style::Color;

    #[test]
    fn bg_shifts_named_codes_and_rewrites_extended_ones() {
        assert_eq!(bg(Some(Color::DarkBlue)), "\x1b[44m");
        assert_eq!(bg(Some(Color::Cyan)), "\x1b[106m");
        assert_eq!(bg(Some(Color::Reset)), "\x1b[49m");
        assert_eq!(bg(Some(Color::AnsiValue(236))), "\x1b[48;5;236m");
        assert_eq!(
            bg(Some(Color::Rgb { r: 1, g: 2, b: 3 })),
            "\x1b[48;2;1;2;3m"
        );
        assert_eq!(bg(None), "");
    }

    #[test]
    fn fg_matches_the_named_constants() {
        assert_eq!(fg(Some(Color::DarkBlue)), BLUE);
        assert_eq!(fg(Some(Color::DarkGreen)), GREEN);
        assert_eq!(fg(Some(Color::DarkRed)), RED);
        assert_eq!(fg(Some(Color::DarkYellow)), YELLOW);
        assert_eq!(fg(Some(Color::DarkCyan)), CYAN);
    }

    #[test]
    fn fg_encodes_bright_rgb_and_none() {
        assert_eq!(fg(Some(Color::Magenta)), "\x1b[95m");
        assert_eq!(
            fg(Some(Color::Rgb { r: 1, g: 2, b: 3 })),
            "\x1b[38;2;1;2;3m"
        );
        assert_eq!(fg(None), "");
    }
}
