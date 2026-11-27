const PIECE_MAX: usize = 24;
const MIXED_PIECE_MIN: usize = 8;
const HEX_RUN_MIN: usize = 32;
const BASE_CHARS: &str = "._:-";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Charset {
    extra: &'static str,
    total_max: usize,
}

pub const PLAIN: Charset = Charset {
    extra: "",
    total_max: 64,
};

pub const FONT: Charset = Charset {
    extra: " ,\"",
    total_max: 120,
};

pub const FLAG: Charset = Charset {
    extra: "=",
    total_max: 64,
};

fn pieces(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|piece| !piece.is_empty())
}

fn is_mixed_piece(piece: &str) -> bool {
    piece.len() >= MIXED_PIECE_MIN
        && piece.chars().any(|c| c.is_ascii_alphabetic())
        && piece.chars().any(|c| c.is_ascii_digit())
}

fn has_long_hex_run(text: &str) -> bool {
    let mut run = 0;
    for piece in pieces(text) {
        if piece.chars().all(|c| c.is_ascii_hexdigit()) {
            run += piece.len();
            if run >= HEX_RUN_MIN {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

pub fn pieces_ok(text: &str) -> bool {
    pieces(text).all(|piece| piece.len() <= PIECE_MAX && !is_mixed_piece(piece))
        && !has_long_hex_run(text)
}

fn char_ok(charset: Charset, c: char) -> bool {
    c.is_ascii_alphanumeric() || BASE_CHARS.contains(c) || charset.extra.contains(c)
}

pub fn text_ok(charset: Charset, text: &str) -> bool {
    text.len() <= charset.total_max && text.chars().all(|c| char_ok(charset, c)) && pieces_ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_base_charset_is_letters_digits_and_four_separators() {
        for text in ["", "a", "A1", "a.b_c:d-e", "x:1", "base16-ocean.dark"] {
            assert!(text_ok(PLAIN, text), "{text}");
        }
        for text in [
            "a b",
            "a\"b",
            "a,b",
            "a=b",
            "a/b",
            "a@b",
            "a!",
            "#a",
            "a*",
            "a|b",
            "a\u{200B}b",
            "é",
            "a\\b",
            "a~b",
            "a$b",
            "a%b",
        ] {
            assert!(!text_ok(PLAIN, text), "{text}");
        }
    }

    #[test]
    fn extra_characters_belong_to_one_charset_only() {
        assert!(text_ok(FONT, "\"Geist Mono\", monospace"));
        assert!(!text_ok(FONT, "a=b"));
        assert!(!text_ok(FONT, "a;b"));
        assert!(text_ok(FLAG, "--color=always"));
        assert!(!text_ok(FLAG, "a b"));
        assert!(!text_ok(FLAG, "\"a\""));
        assert!(!text_ok(PLAIN, "--color=always"));
        assert!(!text_ok(PLAIN, "\"a\""));
    }

    #[test]
    fn a_piece_may_hold_at_most_24_characters() {
        assert!(text_ok(PLAIN, &"q".repeat(24)));
        assert!(!text_ok(PLAIN, &"q".repeat(25)));
        assert!(!text_ok(PLAIN, &format!("a-{}", "q".repeat(25))));
        assert!(!text_ok(FONT, &format!("a {}", "q".repeat(25))));
    }

    #[test]
    fn the_total_length_is_capped_per_charset() {
        let words = |count: usize| vec!["xyz"; count].join("-");
        assert_eq!(words(16).len(), 63);
        assert!(text_ok(PLAIN, &words(16)));
        assert!(text_ok(PLAIN, &format!("{}-", words(16))));
        assert!(!text_ok(PLAIN, &words(17)));
        assert_eq!(format!("{}-x", words(16)).len(), 65);
        assert!(!text_ok(PLAIN, &format!("{}-x", words(16))));
        assert!(text_ok(FONT, &words(17)));
        assert!(!text_ok(FONT, &words(31)));
        assert!(!text_ok(FLAG, &words(17)));
    }

    #[test]
    fn a_long_piece_that_mixes_letters_and_digits_is_opaque() {
        assert!(!text_ok(PLAIN, "AbCdEf01"));
        assert!(!text_ok(PLAIN, "tok-AbCdEf0123"));
        assert!(text_ok(PLAIN, "AbCdEf0"));
        assert!(text_ok(PLAIN, "20250805"));
        assert!(text_ok(PLAIN, "abcdefgh"));
        assert!(text_ok(PLAIN, "gpt4o-v2"));
    }

    #[test]
    fn a_hex_run_of_32_characters_across_pieces_is_opaque() {
        assert!(!text_ok(PLAIN, "deadbeef-cafebabe-deadbeef-cafebabe"));
        assert!(!text_ok(PLAIN, "id-deadbeef-cafebabe-deadbeef-cafebabe"));
        assert!(!text_ok(PLAIN, "550e8400-e29b-41d4-a716-446655440000"));
        assert!(text_ok(PLAIN, "deadbeef-cafebabe-deadbeef-cafebab"));
        assert!(text_ok(PLAIN, "deadbeef-cafebabe-xy-deadbeef-cafebabe"));
        assert!(text_ok(PLAIN, "2024-08-06-2024-08-06"));
    }

    #[test]
    fn real_model_names_pass() {
        for text in [
            "claude-opus-4-1-20250805",
            "claude-3-5-sonnet-20241022",
            "gpt-4o-2024-08-06",
            "gpt-4.1-mini",
            "o3-mini-2025-01-31",
            "gpt-5.1-codex-max",
            "gemini-2.0-flash-001",
            "anthropic.claude-sonnet-4-20250514-v1:0",
            "tokyo-night",
            "ask-me",
            "task-force",
        ] {
            assert!(text_ok(PLAIN, text), "{text}");
        }
    }

    #[test]
    fn pieces_ok_checks_only_pieces() {
        assert!(pieces_ok("Solarized (dark)"));
        assert!(pieces_ok("a b/c"));
        assert!(!pieces_ok("a AbCdEf0123"));
        assert!(!pieces_ok(&"q".repeat(25)));
    }
}
