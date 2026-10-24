pub use anstream::println as aprintln;

pub mod colors {
    pub const RESET: &str = "\x1b[0m";

    pub const TKN_RED: &str = "\x1b[38;2;247;118;142m";
    pub const TKN_GREEN: &str = "\x1b[38;2;158;206;106m";
    pub const TKN_YELLOW: &str = "\x1b[38;2;224;175;104m";
    pub const TKN_BLUE: &str = "\x1b[38;2;122;162;247m";
    pub const TKN_CYAN: &str = "\x1b[38;2;125;207;255m";
}

/// Print colored output similar to the bash print.sh functions
pub struct ColorPrinter;

impl ColorPrinter {
    /// Print green text
    pub fn green(text: &str) -> String {
        format!("{}{}{}", colors::TKN_GREEN, text, colors::RESET)
    }

    /// Print red text
    pub fn red(text: &str) -> String {
        format!("{}{}{}", colors::TKN_RED, text, colors::RESET)
    }

    /// Print yellow text
    pub fn yellow(text: &str) -> String {
        format!("{}{}{}", colors::TKN_YELLOW, text, colors::RESET)
    }

    /// Print blue text
    pub fn blue(text: &str) -> String {
        format!("{}{}{}", colors::TKN_BLUE, text, colors::RESET)
    }

    /// Print cyan text
    pub fn cyan(text: &str) -> String {
        format!("{}{}{}", colors::TKN_CYAN, text, colors::RESET)
    }
}

/// Convenience functions for colored printing (matching bash p g/r/y/m/b/c)
pub fn p_g(text: &str) -> String {
    ColorPrinter::green(text)
}

pub fn p_r(text: &str) -> String {
    ColorPrinter::red(text)
}

pub fn p_y(text: &str) -> String {
    ColorPrinter::yellow(text)
}

pub fn p_b(text: &str) -> String {
    ColorPrinter::blue(text)
}

pub fn p_c(text: &str) -> String {
    ColorPrinter::cyan(text)
}
