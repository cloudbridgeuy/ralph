pub const ICONS: &[(&str, char)] = &[
    ("Done", '\u{F00C}'),
    ("Failed", '\u{F00D}'),
    ("Warning", '\u{F071}'),
    ("Note", '\u{F05A}'),
    ("Question", '\u{F059}'),
    ("Decision", '\u{F0E3}'),
    ("Next", '\u{F061}'),
];

pub fn icon_for(word: &str) -> Option<char> {
    ICONS
        .iter()
        .find(|(group, _)| *group == word)
        .map(|(_, icon)| *icon)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::pill::PILL_WORDS;

    #[test]
    fn every_pill_word_group_has_an_icon() {
        for entry in PILL_WORDS {
            assert!(icon_for(entry.word).is_some(), "{}", entry.word);
        }
    }

    #[test]
    fn icons_are_font_awesome_codepoints() {
        for (_, icon) in ICONS {
            assert!(('\u{F000}'..='\u{F2FF}').contains(icon), "{icon:?}");
        }
    }

    #[test]
    fn table_has_one_entry_per_group_and_no_extras() {
        assert_eq!(ICONS.len(), PILL_WORDS.len());
        for (word, _) in ICONS {
            assert!(PILL_WORDS.iter().any(|entry| entry.word == *word), "{word}");
        }
    }

    #[test]
    fn icons_match_the_design() {
        let want = [
            ("Done", '\u{F00C}'),
            ("Failed", '\u{F00D}'),
            ("Warning", '\u{F071}'),
            ("Note", '\u{F05A}'),
            ("Question", '\u{F059}'),
            ("Decision", '\u{F0E3}'),
            ("Next", '\u{F061}'),
        ];
        for (word, icon) in want {
            assert_eq!(icon_for(word), Some(icon), "{word}");
        }
        assert_eq!(icon_for("Nope"), None);
    }

    #[test]
    fn icons_are_one_column_wide() {
        use unicode_width::UnicodeWidthChar;
        for (_, icon) in ICONS {
            assert_eq!(icon.width(), Some(1), "{icon:?}");
        }
    }
}
