pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";
pub const RED: &str = "\x1b[31m";
pub const GREEN: &str = "\x1b[32m";
pub const YELLOW: &str = "\x1b[33m";
pub const CYAN: &str = "\x1b[36m";
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
        assert_eq!(CYAN, "\x1b[36m");
        assert_eq!(RESET, "\x1b[0m");
    }
}
