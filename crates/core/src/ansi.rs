pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";
pub const RED: &str = "\x1b[31m";
pub const GREEN: &str = "\x1b[32m";
pub const YELLOW: &str = "\x1b[33m";
pub const BLUE: &str = "\x1b[34m";
pub const MAGENTA: &str = "\x1b[35m";
pub const CYAN: &str = "\x1b[36m";
pub const GREY: &str = "\x1b[90m";
pub const BRIGHT_YELLOW: &str = "\x1b[93m";
pub const RED_BG: &str = "\x1b[48;2;140;45;45m";
pub const GREEN_BG: &str = "\x1b[48;2;45;130;45m";
pub const RESET: &str = "\x1b[0m";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_keep_their_byte_sequences() {
        assert_eq!(BOLD, "\x1b[1m");
        assert_eq!(DIM, "\x1b[2m");
        assert_eq!(RED, "\x1b[31m");
        assert_eq!(GREEN, "\x1b[32m");
        assert_eq!(YELLOW, "\x1b[33m");
        assert_eq!(BLUE, "\x1b[34m");
        assert_eq!(MAGENTA, "\x1b[35m");
        assert_eq!(CYAN, "\x1b[36m");
        assert_eq!(GREY, "\x1b[90m");
        assert_eq!(BRIGHT_YELLOW, "\x1b[93m");
        assert_eq!(RED_BG, "\x1b[48;2;140;45;45m");
        assert_eq!(GREEN_BG, "\x1b[48;2;45;130;45m");
        assert_eq!(RESET, "\x1b[0m");
    }
}
