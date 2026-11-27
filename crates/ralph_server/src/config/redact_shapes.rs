mod charset;

use charset::{FLAG, FONT, PLAIN};
use serde_json::Value;

const VIEW_VALUES: &[&str] = &["table", "list"];
const PR_PROVIDER_VALUES: &[&str] = &["github", "none"];
const WORD_MAX: usize = 24;
const NAME_MAX: usize = 40;
const NUMBER_MAX: usize = 12;
const GRAMMAR_KEY_PREFIXES: [&str; 5] = [
    "theme.",
    "mermaid.style.colors.",
    "mermaid.style.config.",
    "wire.prices.",
    "tickets.",
];
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
    Font,
    Flag,
    Number,
}

pub fn key_class(key: &str) -> Class {
    match key {
        "workers.view" => Class::Enum(VIEW_VALUES),
        "workers.pr_provider" => Class::Enum(PR_PROVIDER_VALUES),
        "theme.name" | "theme.agent" | "theme.assistant.code" | "theme.agent.code" => Class::Name,
        "mermaid.style.palette" | "mermaid.style.curve" => Class::Word,
        "web.font" | "mermaid.style.font_family" | "mermaid.style.font_family_mono" => Class::Font,
        "changes.diff_args" => Class::Flag,
        "web.ansi_dark" | "web.ansi_light" => Class::Colour,
        "chat.reset_turns" | "chat.reset_tokens" | "chat.worker_quiet_secs" => Class::Number,
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
        && charset::pieces_ok(text)
}

pub fn is_name(text: &str) -> bool {
    text.len() <= NAME_MAX
        && text.starts_with(|c: char| c.is_ascii_alphanumeric())
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '(' | ')' | '-'))
        && charset::pieces_ok(text)
}

pub fn is_ident(text: &str) -> bool {
    !text.is_empty() && charset::text_ok(PLAIN, text)
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
        Class::Loose => charset::text_ok(PLAIN, text),
        Class::Font => charset::text_ok(FONT, text),
        Class::Flag => charset::text_ok(FLAG, text),
        Class::Number => false,
    }
}

pub fn value_shape_ok(class: Class, value: &Value) -> bool {
    if class == Class::Number {
        return value.is_number();
    }
    match value {
        Value::String(text) => string_shape_ok(class, text),
        Value::Array(items) => items.iter().all(|item| value_shape_ok(class, item)),
        Value::Object(map) => map
            .iter()
            .all(|(key, item)| is_ident(key) && value_shape_ok(class, item)),
        _ => true,
    }
}

pub fn key_shape_ok(key: &str) -> bool {
    GRAMMAR_KEY_PREFIXES
        .iter()
        .find_map(|prefix| key.strip_prefix(prefix))
        .is_none_or(|rest| !rest.split('.').any(str::is_empty) && is_ident(rest))
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
        for text in ["/Users/x", "~/notes", "D:/x", "a/b", "~", "a~b"] {
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
        assert_eq!(key_class("mermaid.style.font_family"), Class::Font);
        assert_eq!(key_class("mermaid.style.font_family_mono"), Class::Font);
        assert_eq!(key_class("changes.diff_args"), Class::Flag);
        assert_eq!(key_class("web.ansi_dark"), Class::Colour);
        assert_eq!(key_class("tickets.linear"), Class::Ident);
        assert_eq!(key_class("workers.view"), Class::Enum(VIEW_VALUES));
        assert_eq!(key_class("web.font"), Class::Font);
        assert_eq!(key_class("claude.model"), Class::Loose);
        assert_eq!(key_class("chat.reset_turns"), Class::Number);
        assert_eq!(key_class("chat.reset_tokens"), Class::Number);
        assert_eq!(key_class("chat.worker_quiet_secs"), Class::Number);
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
        assert!(!is_word("AbCdEf0123") && !is_word("a-AbCdEf0123"));
        assert!(!is_word("a_b") && !is_word("a b") && !is_word("1a") && !is_word(&"a".repeat(25)));
        assert!(
            is_name("tokyo-night") && is_name("Solarized (dark)") && is_name("base16-ocean.dark")
        );
        assert!(!is_name("ghp_SECRET") && !is_name("a@b") && !is_name(&"a".repeat(41)));
        assert!(is_ident("claude-haiku-5-5") && is_ident("x:1") && is_ident("gpt-4.1"));
        assert!(
            !is_ident("") && !is_ident("a b") && !is_ident("a\"b") && !is_ident(&"a".repeat(65))
        );
        assert!(is_ident(&["xyz"; 16].join("-")));
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
        assert!(value_shape_ok(Class::Loose, &json!("anything-at-all")));
        assert!(!value_shape_ok(Class::Loose, &json!("anything at all")));
        assert!(value_shape_ok(Class::Font, &json!(["Geist Mono", "\"a\""])));
        assert!(!value_shape_ok(Class::Font, &json!("a=b")));
        assert!(value_shape_ok(Class::Flag, &json!(["--color=always"])));
        assert!(!value_shape_ok(Class::Flag, &json!(["a b"])));
    }

    #[test]
    fn the_number_class_accepts_only_numbers() {
        assert!(value_shape_ok(Class::Number, &json!(1)));
        assert!(value_shape_ok(Class::Number, &json!(2.5)));
        for value in [
            json!("1"),
            json!("my password is hunter2"),
            json!(["my secret words"]),
            json!([1]),
            json!({"a": 1}),
            json!(true),
            json!(null),
        ] {
            assert!(!value_shape_ok(Class::Number, &value), "{value}");
        }
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
            "mermaid.style.colors.",
        ] {
            assert!(!key_shape_ok(key), "{key}");
        }
        assert!(!key_shape_ok(&format!("theme.status.{}", "a".repeat(25))));
        assert!(key_shape_ok(&format!("theme.status.{}", "a".repeat(24))));
        assert!(key_shape_ok("mermaid.style.colors.ghp:x"));
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

    const GHP: &str = "ghp_AbCdEf0123456789AbCdEf0123456789abcd";
    const HEX40: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b";
    const LETTERS40: &str = "QWERTYUIOPASDFGHJKLZXCVBNMQWERTYUIOPASDF";

    #[test]
    fn opaque_row_key_segments_are_not_key_shaped() {
        for key in [
            format!("theme.status.{GHP}"),
            format!("mermaid.style.colors.{HEX40}"),
            format!("mermaid.style.config.x.{HEX40}.{HEX40}"),
            format!("theme.status.{LETTERS40}"),
            format!("wire.prices.{GHP}.input"),
        ] {
            assert!(!key_shape_ok(&key), "{key}");
        }
    }

    #[test]
    fn opaque_object_keys_fail_at_any_depth() {
        for value in [
            json!({ GHP: 1 }),
            json!([{ GHP: 1 }]),
            json!({"a": [{"b": {HEX40: "x"}}]}),
            json!([{ LETTERS40: 1 }]),
        ] {
            assert!(!value_shape_ok(Class::Free, &value), "{value}");
            assert!(!value_shape_ok(Class::Colour, &value), "{value}");
        }
    }

    #[test]
    fn real_keys_stay_key_shaped() {
        for key in [
            "theme.status.base_bg",
            "theme.assistant.code",
            "theme.diff.add_fg",
            "mermaid.style.colors.accent",
            "mermaid.style.config.flowchart.nodeSpacing",
            "mermaid.style.config.themeVariables.fontSize",
            "wire.prices.claude-opus-4-1-20250805.input",
            "wire.prices.gpt-4o-2024-08-06.output",
            "wire.prices.gpt-4.1-mini.input",
            "wire.prices.o3-mini-2025-01-31.input",
        ] {
            assert!(key_shape_ok(key), "{key}");
        }
        assert!(value_shape_ok(
            Class::Free,
            &json!([{"flowchart": {"font_size": 14}}])
        ));
    }

    #[test]
    fn a_dotted_key_counts_as_one_string_against_the_cap() {
        let sixty_four = format!("{}-", ["xyz"; 16].join("-"));
        assert_eq!(sixty_four.len(), 64);
        assert!(key_shape_ok(&format!("mermaid.style.colors.{sixty_four}")));
        assert!(!key_shape_ok(&format!(
            "mermaid.style.colors.{sixty_four}x"
        )));
        let dotted = ["abcdefghijklmnopqrst"; 3].join(".");
        assert_eq!(dotted.len(), 62);
        assert!(key_shape_ok(&format!("mermaid.style.config.{dotted}")));
        assert!(!key_shape_ok(&format!("mermaid.style.config.{dotted}.ab")));
    }

    #[test]
    fn code_theme_fields_are_names() {
        for key in ["theme.assistant.code", "theme.agent.code", "theme.agent"] {
            assert_eq!(key_class(key), Class::Name, "{key}");
        }
        assert_eq!(key_class("theme.assistant.heading1"), Class::Colour);
        assert_eq!(key_class("theme.agent.border"), Class::Colour);
        assert_eq!(key_class("theme.agent.codex"), Class::Colour);
    }

    #[test]
    fn ticket_provider_keys_use_the_shared_key_check() {
        assert!(key_shape_ok("tickets.linear"));
        assert!(key_shape_ok("tickets.jira"));
        assert!(!key_shape_ok(&format!("tickets.{GHP}")));
        assert!(!key_shape_ok("tickets.my secret words"));
    }
}
