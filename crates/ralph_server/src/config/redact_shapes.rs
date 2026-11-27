use serde_json::Value;

const VIEW_VALUES: &[&str] = &["table", "list"];
const PR_PROVIDER_VALUES: &[&str] = &["github", "none"];
const WORD_MAX: usize = 24;
const NAME_MAX: usize = 40;
const IDENT_MAX: usize = 64;
const NUMBER_MAX: usize = 12;
const KEY_MAX: usize = 40;
const GRAMMAR_KEY_PREFIXES: [&str; 3] =
    ["theme.", "mermaid.style.colors.", "mermaid.style.config."];
const COLOUR_FUNCTIONS: [&str; 4] = ["rgb", "rgba", "hsl", "hsla"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Colour,
    Name,
    Word,
    Ident,
    Enum(&'static [&'static str]),
    Free,
    Loose,
}

pub fn key_class(key: &str) -> Class {
    match key {
        "workers.view" => Class::Enum(VIEW_VALUES),
        "workers.pr_provider" => Class::Enum(PR_PROVIDER_VALUES),
        "theme.name" => Class::Name,
        "mermaid.style.palette" | "mermaid.style.curve" => Class::Word,
        "mermaid.style.font_family" | "mermaid.style.font_family_mono" => Class::Loose,
        "web.ansi_dark" | "web.ansi_light" => Class::Colour,
        _ if key == "mermaid.style.config" || key.starts_with("mermaid.style.config.") => {
            Class::Free
        }
        _ if key.starts_with("theme.") || key.starts_with("mermaid.style.") => Class::Colour,
        _ if key.starts_with("tickets.") => Class::Ident,
        _ => Class::Loose,
    }
}

fn is_hex_colour(text: &str) -> bool {
    text.strip_prefix('#').is_some_and(|digits| {
        matches!(digits.len(), 3 | 4 | 6 | 8) && digits.chars().all(|c| c.is_ascii_hexdigit())
    })
}

fn is_functional_colour(text: &str) -> bool {
    let Some((name, rest)) = text.split_once('(') else {
        return false;
    };
    let Some(inner) = rest.strip_suffix(')') else {
        return false;
    };
    COLOUR_FUNCTIONS.contains(&name)
        && !inner.is_empty()
        && inner
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | '%' | ',' | ' '))
}

fn is_colour_word(text: &str) -> bool {
    text.len() <= WORD_MAX
        && text.starts_with(|c: char| c.is_ascii_lowercase())
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || matches!(c, '_' | '-'))
}

fn is_ansi_index(text: &str) -> bool {
    (1..=3).contains(&text.len()) && text.chars().all(|c| c.is_ascii_digit())
}

pub fn is_colour(text: &str) -> bool {
    is_hex_colour(text) || is_functional_colour(text) || is_colour_word(text) || is_ansi_index(text)
}

pub fn is_word(text: &str) -> bool {
    text.len() <= WORD_MAX
        && text.starts_with(|c: char| c.is_ascii_alphabetic())
        && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

pub fn is_name(text: &str) -> bool {
    text.len() <= NAME_MAX
        && text.starts_with(|c: char| c.is_ascii_alphanumeric())
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '(' | ')' | '-'))
}

pub fn is_ident(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= IDENT_MAX
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'))
}

pub fn is_key_ident(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= KEY_MAX
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
}

fn is_plain_number(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= NUMBER_MAX
        && text.chars().all(|c| c.is_ascii_digit() || c == '.')
}

fn string_shape_ok(class: Class, text: &str) -> bool {
    match class {
        Class::Colour => is_colour(text),
        Class::Name => is_name(text),
        Class::Word => is_word(text),
        Class::Ident => is_ident(text),
        Class::Enum(allowed) => allowed.contains(&text),
        Class::Free => is_colour(text) || is_word(text) || is_plain_number(text),
        Class::Loose => true,
    }
}

pub fn value_shape_ok(class: Class, value: &Value) -> bool {
    match value {
        Value::String(text) => string_shape_ok(class, text),
        Value::Array(items) => items.iter().all(|item| value_shape_ok(class, item)),
        Value::Object(map) => map
            .iter()
            .all(|(key, item)| is_key_ident(key) && value_shape_ok(class, item)),
        _ => true,
    }
}

pub fn key_shape_ok(key: &str) -> bool {
    if let Some(rest) = key.strip_prefix("wire.prices.") {
        return rest.split('.').all(is_ident);
    }
    GRAMMAR_KEY_PREFIXES
        .iter()
        .find_map(|prefix| key.strip_prefix(prefix))
        .is_none_or(|rest| rest.split('.').all(is_key_ident))
}

const TOKEN_PREFIXES: [&str; 8] = [
    "sk-",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "github_pat_",
    "bearer ",
];
const AWS_PREFIXES: [&str; 2] = ["AKIA", "ASIA"];
const AWS_BODY_LEN: usize = 16;
const OPAQUE_MIN: usize = 20;
const OPAQUE_AFTER_PREFIX_MIN: usize = 16;
const UUID_PART_LENS: [usize; 5] = [8, 4, 4, 4, 12];

fn at_boundary(text: &str, index: usize) -> bool {
    !text[..index]
        .chars()
        .next_back()
        .is_some_and(|c| c.is_ascii_alphanumeric())
}

fn boundary_matches<'a>(text: &'a str, needle: &'a str) -> impl Iterator<Item = &'a str> {
    text.match_indices(needle)
        .filter(|(index, _)| at_boundary(text, *index))
        .map(|(index, _)| &text[index + needle.len()..])
}

fn has_token_prefix(lowered: &str) -> bool {
    TOKEN_PREFIXES
        .iter()
        .any(|prefix| boundary_matches(lowered, prefix).next().is_some())
}

fn has_jwt(text: &str) -> bool {
    let Some((second_last_dot, _)) = text.rmatch_indices('.').nth(1) else {
        return false;
    };
    text.match_indices("eyJ")
        .any(|(index, _)| at_boundary(text, index) && index + "eyJ".len() <= second_last_dot)
}

fn has_aws_key_id(text: &str) -> bool {
    AWS_PREFIXES.iter().any(|prefix| {
        boundary_matches(text, prefix).any(|rest| {
            let body: Vec<char> = rest.chars().take(AWS_BODY_LEN).collect();
            body.len() == AWS_BODY_LEN
                && body
                    .iter()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        })
    })
}

fn has_slack_token(lowered: &str) -> bool {
    boundary_matches(lowered, "xox").any(|rest| {
        let mut chars = rest.chars();
        chars.next().is_some_and(|c| c.is_ascii_lowercase()) && chars.next() == Some('-')
    })
}

fn is_opaque_run(segment: &str, min: usize) -> bool {
    segment.len() >= min
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '_' | '='))
        && segment.chars().any(|c| c.is_ascii_alphabetic())
        && segment.chars().any(|c| c.is_ascii_digit())
}

fn is_alphabetic_word(segment: &str) -> bool {
    !segment.is_empty() && segment.chars().all(|c| c.is_ascii_alphabetic())
}

fn underscore_prefixed_run(segment: &str) -> bool {
    segment.split_once('_').is_some_and(|(prefix, rest)| {
        is_alphabetic_word(prefix) && is_opaque_run(rest, OPAQUE_AFTER_PREFIX_MIN)
    })
}

fn is_hard_separator(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '"' | '\'' | '`' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>'
        )
}

fn is_joiner(c: char) -> bool {
    matches!(c, '-' | '.' | ':')
}

fn is_piece_separator(c: char) -> bool {
    is_hard_separator(c) || is_joiner(c)
}

fn char_class(c: char) -> u8 {
    if c.is_ascii_uppercase() {
        0
    } else if c.is_ascii_lowercase() {
        1
    } else if c.is_ascii_digit() {
        2
    } else {
        3
    }
}

fn class_changes(text: &str) -> usize {
    text.chars()
        .zip(text.chars().skip(1))
        .filter(|(a, b)| char_class(*a) != char_class(*b))
        .count()
}

fn is_noisy_piece(piece: &str) -> bool {
    !piece.is_empty()
        && piece.chars().all(|c| c.is_ascii_alphanumeric())
        && (piece.chars().any(|c| c.is_ascii_digit())
            || piece
                .chars()
                .zip(piece.chars().skip(1))
                .any(|(a, b)| !a.is_ascii_uppercase() && b.is_ascii_uppercase()))
}

fn is_mixed_case_with_digits(text: &str) -> bool {
    text.chars().any(|c| c.is_ascii_uppercase())
        && text.chars().any(|c| c.is_ascii_lowercase())
        && text.chars().any(|c| c.is_ascii_digit())
}

fn is_dense_run(text: &str) -> bool {
    class_changes(text) * 5 >= text.len() * 2
}

fn is_uuid(text: &str) -> bool {
    let parts: Vec<&str> = text.split('-').collect();
    parts.len() == UUID_PART_LENS.len()
        && parts
            .iter()
            .zip(UUID_PART_LENS)
            .all(|(part, len)| part.len() == len && part.chars().all(|c| c.is_ascii_hexdigit()))
}

fn has_joined_opaque_run(token: &str) -> bool {
    if token.split(['.', ':']).any(is_uuid) {
        return true;
    }
    let pieces: Vec<&str> = token.split(is_joiner).collect();
    pieces.split(|piece| !is_noisy_piece(piece)).any(|run| {
        let joined = run.concat();
        run.len() >= 2
            && joined.len() >= OPAQUE_MIN
            && is_mixed_case_with_digits(&joined)
            && is_dense_run(&joined)
    })
}

fn has_opaque_run(text: &str) -> bool {
    let mut previous = "";
    for segment in text.split(is_piece_separator) {
        if is_opaque_run(segment, OPAQUE_MIN)
            || underscore_prefixed_run(segment)
            || (is_alphabetic_word(previous) && is_opaque_run(segment, OPAQUE_AFTER_PREFIX_MIN))
        {
            return true;
        }
        previous = segment;
    }
    text.split(is_hard_separator).any(has_joined_opaque_run)
}

pub fn text_has_secret_shape(text: &str) -> bool {
    let lowered = text.to_ascii_lowercase();
    text.contains("-----BEGIN")
        || has_token_prefix(&lowered)
        || has_jwt(text)
        || has_aws_key_id(text)
        || has_slack_token(&lowered)
        || has_opaque_run(text)
}

fn is_variable_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn has_percent_variable(text: &str) -> bool {
    text.match_indices('%').any(|(index, _)| {
        let rest = &text[index + 1..];
        let name_len = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        name_len > 0 && rest.starts_with(is_variable_start) && rest[name_len..].starts_with('%')
    })
}

fn has_dollar_variable(text: &str) -> bool {
    text.match_indices('$').any(|(index, _)| {
        text[index + 1..]
            .chars()
            .next()
            .is_some_and(|c| is_variable_start(c) || c == '{')
    })
}

pub fn text_has_path_shape(text: &str) -> bool {
    text.contains(['/', '~', '\\']) || has_percent_variable(text) || has_dollar_variable(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn backslash_and_unc_paths_have_a_path_shape() {
        for text in [r"C:\Users\alice", r"\\host\share", r"a\b", r"C:\x"] {
            assert!(text_has_path_shape(text), "{text}");
        }
    }

    #[test]
    fn unix_and_drive_slash_paths_have_a_path_shape() {
        for text in ["/Users/x", "~/notes", "D:/x", "a/b"] {
            assert!(text_has_path_shape(text), "{text}");
        }
    }

    #[test]
    fn env_variable_references_have_a_path_shape() {
        for text in [
            "%USERPROFILE%",
            "%APPDATA%x",
            "$HOME",
            "${HOME}",
            "$HOME_DIR",
        ] {
            assert!(text_has_path_shape(text), "{text}");
        }
    }

    #[test]
    fn key_class_maps_tight_and_loose_fields() {
        assert_eq!(key_class("theme.status.base_bg"), Class::Colour);
        assert_eq!(key_class("theme.name"), Class::Name);
        assert_eq!(key_class("mermaid.style.colors.bg"), Class::Colour);
        assert_eq!(key_class("mermaid.style.series"), Class::Colour);
        assert_eq!(key_class("mermaid.style.curve"), Class::Word);
        assert_eq!(key_class("mermaid.style.config"), Class::Free);
        assert_eq!(key_class("mermaid.style.config.flowchart"), Class::Free);
        assert_eq!(key_class("mermaid.style.font_family"), Class::Loose);
        assert_eq!(key_class("web.ansi_dark"), Class::Colour);
        assert_eq!(key_class("tickets.linear"), Class::Ident);
        assert_eq!(key_class("workers.view"), Class::Enum(VIEW_VALUES));
        assert_eq!(key_class("web.font"), Class::Loose);
        assert_eq!(key_class("claude.model"), Class::Loose);
    }

    #[test]
    fn colour_grammar_accepts_colours_only() {
        for text in [
            "#fff",
            "#ffff",
            "#ff00aa",
            "#ff00aa80",
            "rgb(1,2,3)",
            "rgba(1, 2, 3, 0.5)",
            "hsl(10, 50%, 50%)",
            "hsla(10,50%,50%,.5)",
            "dark_green",
            "reset",
            "236",
        ] {
            assert!(is_colour(text), "{text}");
        }
        for text in [
            "#ff",
            "#ff00a",
            "#gggggg",
            "rgb()",
            "rgb(1,2,ghp)",
            "rgb(/etc)",
            "ghp_SECRET",
            "Hunter2",
            "two words",
            "",
            "1234",
            "foo(1)",
        ] {
            assert!(!is_colour(text), "{text}");
        }
        assert!(!is_colour(&"a".repeat(25)));
    }

    #[test]
    fn word_name_and_ident_grammars_bound_length_and_charset() {
        assert!(is_word("basis") && is_word("step-after") && is_word("monotoneX"));
        assert!(!is_word("a_b") && !is_word("a b") && !is_word("1a") && !is_word(&"a".repeat(25)));
        assert!(
            is_name("tokyo-night") && is_name("Solarized (dark)") && is_name("base16-ocean.dark")
        );
        assert!(!is_name("ghp_SECRET") && !is_name("a@b") && !is_name(&"a".repeat(41)));
        assert!(is_ident("claude-haiku-5-5") && is_ident("x:1") && is_ident("gpt-4.1"));
        assert!(
            !is_ident("") && !is_ident("a b") && !is_ident("a\"b") && !is_ident(&"a".repeat(65))
        );
        assert!(is_ident(&"a".repeat(64)));
    }

    #[test]
    fn value_shape_walks_arrays_and_objects() {
        assert!(value_shape_ok(Class::Colour, &json!(["#fff", "red"])));
        assert!(!value_shape_ok(Class::Colour, &json!(["#fff", "Red!"])));
        assert!(value_shape_ok(
            Class::Free,
            &json!({"a": {"b": ["dark", 3, true, null, "30"]}})
        ));
        assert!(!value_shape_ok(
            Class::Free,
            &json!({"a": {"b": "two words"}})
        ));
        assert!(value_shape_ok(Class::Colour, &json!(96)));
        assert!(value_shape_ok(Class::Enum(VIEW_VALUES), &json!("list")));
        assert!(!value_shape_ok(Class::Enum(VIEW_VALUES), &json!("grid")));
        assert!(value_shape_ok(Class::Loose, &json!("anything at all")));
    }

    #[test]
    fn price_keys_need_ident_segments() {
        assert!(key_shape_ok("wire.prices.gpt-4.1.input"));
        assert!(key_shape_ok("wire.prices.x:1.input"));
        assert!(key_shape_ok("chat.wrap_width"));
        assert!(!key_shape_ok("wire.prices.a b.input"));
        assert!(!key_shape_ok("wire.prices..input"));
    }

    #[test]
    fn grammar_keys_need_ident_segments_under_theme_and_mermaid_maps() {
        for key in [
            "theme.status.base_bg",
            "theme.name",
            "theme.assistant.code",
            "mermaid.style.colors.accent",
            "mermaid.style.config.flowchart.nodeSpacing",
            "mermaid.style.font_size",
        ] {
            assert!(key_shape_ok(key), "{key}");
        }
        for key in [
            "mermaid.style.colors.hunter2 secret",
            "theme.status.my password is hunter2",
            "mermaid.style.config.flowchart.correct horse battery staple",
            "mermaid.style.colors.a\"b",
            "mermaid.style.colors.ghp:x",
            "mermaid.style.colors.",
        ] {
            assert!(!key_shape_ok(key), "{key}");
        }
        assert!(!key_shape_ok(&format!("theme.status.{}", "a".repeat(41))));
        assert!(key_shape_ok(&format!("theme.status.{}", "a".repeat(40))));
    }

    #[test]
    fn value_shape_checks_object_keys_inside_arrays_and_objects() {
        assert!(!value_shape_ok(
            Class::Free,
            &json!([{"correct horse battery": 1}])
        ));
        assert!(!value_shape_ok(Class::Free, &json!({"a": [{"b c": 1}]})));
        assert!(!value_shape_ok(Class::Loose, &json!([{"x y": 1}])));
        assert!(value_shape_ok(
            Class::Free,
            &json!([{"nodeSpacing": 1, "curve_x": "basis"}])
        ));
    }

    #[test]
    fn punctuated_opaque_tokens_are_detected() {
        for text in [
            "pass:AbCdEf0123456789xyzQ",
            "AbCdEf0123456789xyzQ.",
            "\"AbCdEf0123456789xyzQ\"",
            "(AbCdEf0123456789xyzQ)",
            "a,AbCdEf0123456789xyzQ;",
        ] {
            assert!(text_has_secret_shape(text), "{text}");
        }
    }

    #[test]
    fn split_opaque_tokens_are_detected_when_joined() {
        for text in [
            "Zx9k2Lq-8vB3mN0pQrS-7tUwXyZ1aB2cD3e",
            "550e8400-e29b-41d4-a716-446655440000",
            "550E8400-E29B-41D4-A716-446655440000",
            "id:550e8400-e29b-41d4-a716-446655440000",
            "AbCdEf0123.456789xyzQAbCd",
            "wire.prices.AbCdEf0123456.789xyzQAbCd.input",
            "AbCdEf0123:456789xyzQAbCd",
        ] {
            assert!(text_has_secret_shape(text), "{text}");
        }
    }

    #[test]
    fn joined_runs_that_read_as_names_stay_visible() {
        for text in [
            "claude-opus-4-1-20250805",
            "gpt-4.1-mini",
            "gpt-4o-2024-08-06",
            "GPT-4o-2024-08-06-Preview",
            "base16-ocean.dark",
            "1.2.3",
            "v1.2.3-beta.4",
            "ClaudeOpus41-Preview-Latest",
            "Tokyo-Night-Storm-2024-Edition",
            "2024-08-06T12:30:45.123Z",
            "ask-me",
            "task-force",
            "550e8400-e29b-41d4-a716",
            "wire.prices.claude-opus-4-1-20250805.cache_read",
        ] {
            assert!(!text_has_secret_shape(text), "{text}");
        }
    }

    #[test]
    fn token_prefixes_match_in_any_case() {
        for text in [
            "bearer abc",
            "BEARER abc",
            "Authorization: bearer xyz",
            "GHP_secret",
            "Github_Pat_x",
            "SK-ant-abc",
            "XOXB-1-2",
            "Xoxp-1",
        ] {
            assert!(text_has_secret_shape(text), "{text}");
        }
    }

    #[test]
    fn mixed_case_words_that_merely_contain_a_prefix_stay_visible() {
        for text in [
            "ask-me",
            "ASK-ME",
            "Task-Force",
            "TASK-FORCE",
            "xox",
            "BEARERS",
            "eyj.a.b",
        ] {
            assert!(!text_has_secret_shape(text), "{text}");
        }
    }

    #[test]
    fn the_jwt_marker_stays_case_exact() {
        assert!(text_has_secret_shape("eyJhbGciOiJIUzI1NiJ9.eyJzdWIi.sig"));
        assert!(!text_has_secret_shape("EYJhbGciOi.a.b"));
    }

    #[test]
    fn jwt_scan_handles_many_markers_without_dots() {
        let text = "eyJ ".repeat(50_000);
        assert!(!text_has_secret_shape(&text));
        let with_dots = format!("{text}x.y.z");
        assert!(text_has_secret_shape(&with_dots));
    }

    #[test]
    fn a_jwt_needs_two_dots_after_the_marker() {
        assert!(has_jwt("eyJa.b.c"));
        assert!(has_jwt("x.y eyJa.b.c"));
        assert!(!has_jwt("a.b.c eyJx.y"));
        assert!(!has_jwt("eyJa.b"));
    }

    #[test]
    fn aws_key_ids_without_digits_are_caught_by_the_aws_rule_alone() {
        for text in [
            "AKIAABCDEFGHIJKLMNOP",
            "ASIAABCDEFGHIJKLMNOP",
            "id AKIAABCDEFGHIJKLMNOP",
        ] {
            assert!(text_has_secret_shape(text), "{text}");
        }
        for text in [
            "AKIAABCDEFGHIJKLMNO",
            "AKIAabcdefghijklmnop",
            "AKIAABCDEFGHIJKLMNO-",
        ] {
            assert!(!text_has_secret_shape(text), "{text}");
        }
    }

    #[test]
    fn underscore_prefixed_runs_under_twenty_characters_are_caught() {
        assert!(text_has_secret_shape("x_AbCdEf0123456789"));
        assert!(text_has_secret_shape("x_AbCdEf012345678a"));
        assert!(!text_has_secret_shape("x_AbCdEf012345678"));
        assert!(!text_has_secret_shape("x1_AbCdEf0123456789"));
    }

    #[test]
    fn opaque_min_is_twenty() {
        assert!(!text_has_secret_shape("AbCdEf0123456789xyz"));
        assert!(text_has_secret_shape("AbCdEf0123456789xyzQ"));
    }

    #[test]
    fn opaque_after_prefix_min_is_sixteen() {
        assert!(!text_has_secret_shape("token AbCdEf012345678"));
        assert!(text_has_secret_shape("token AbCdEf0123456789"));
        assert!(!text_has_secret_shape("token1 AbCdEf0123456789"));
    }

    #[test]
    fn markers_inside_a_longer_word_are_not_secrets() {
        assert!(text_has_secret_shape("a eyJa.b.c"));
        assert!(!text_has_secret_shape("aeyJa.b.c"));
        assert!(text_has_secret_shape("a AKIAABCDEFGHIJKLMNOP"));
        assert!(!text_has_secret_shape("aAKIAABCDEFGHIJKLMNOP"));
        assert!(text_has_secret_shape("a xo\x78b-1"));
        assert!(!text_has_secret_shape("axo\x78b-1"));
        assert!(text_has_secret_shape("(xo\x78b-1)"));
    }

    #[test]
    fn each_secret_shape_is_detected() {
        for text in [
            "Bearer abc",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.sig",
            "AKIAIOSFODNN7EXAMPLE",
            "ASIAIOSFODNN7EXAMPLE",
            "xo\x78b-1-2",
            "sk-ant-abc",
            "ghp_SECRET",
            "gho_a",
            "ghu_a",
            "ghs_a",
            "ghr_a",
            "github_pat_a",
            "-----BEGIN RSA PRIVATE KEY-----",
            "aGVsbG8gd29ybGQgc2VjcmV0MTI1",
            "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
            "tok_AbCdEf0123456789xyzQ",
            "myapp-AbCdEf0123456789",
        ] {
            assert!(text_has_secret_shape(text), "{text}");
        }
    }

    #[test]
    fn ordinary_text_has_no_secret_shape() {
        for text in [
            "claude-opus-4-1-20250805",
            "claude-sonnet-4-5",
            "gpt-5.1-codex-max",
            "ask-me",
            "task-force",
            "tokyo-night",
            "\"Geist Mono\", monospace",
            "--color=always",
            "AKIASHORT",
            "xox",
            "eyJ.one",
            "averyveryverylongwordwithoutdigits",
            "x:1",
            "96",
        ] {
            assert!(!text_has_secret_shape(text), "{text}");
        }
    }

    #[test]
    fn plain_text_has_no_path_shape() {
        for text in [
            "x:1",
            "C:foo",
            "tokyo-night",
            "claude-opus-4-1-20250805",
            "50%",
            "hsl(10, 50%, 50%)",
            "cost $5",
            "100%",
        ] {
            assert!(!text_has_path_shape(text), "{text}");
        }
    }
}
