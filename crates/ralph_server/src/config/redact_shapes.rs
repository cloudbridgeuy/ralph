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
