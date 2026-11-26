use super::core::Screen;

pub const PLACEHOLDER: &str = "--";

const AGO_PATTERN: &str = r"\d+[a-z]+ ago";

const AGO_MASK_TEMPLATE: &str = r"const walker = document.createTreeWalker(document.getElementById('main'), NodeFilter.SHOW_TEXT); const nodes = []; while (walker.nextNode()) nodes.push(walker.currentNode); nodes.forEach((node) => { node.nodeValue = node.nodeValue.replace(/AGO_PATTERN/g, 'PLACEHOLDER'); }); return nodes.length;";

pub fn mask_script(screen: Screen) -> Option<String> {
    match screen {
        Screen::Threads => Some(
            AGO_MASK_TEMPLATE
                .replace("AGO_PATTERN", AGO_PATTERN)
                .replace("PLACEHOLDER", PLACEHOLDER),
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn masked(text: &str) -> String {
        let mut out = String::new();
        let mut rest = text;
        while let Some(found) = rest.find(" ago") {
            let head = &rest[..found];
            let letters = head
                .trim_end_matches(|c: char| c.is_ascii_lowercase())
                .len();
            let digits = head[..letters]
                .trim_end_matches(|c: char| c.is_ascii_digit())
                .len();
            if letters < head.len() && digits < letters {
                out.push_str(&head[..digits]);
                out.push_str(PLACEHOLDER);
            } else {
                out.push_str(head);
                out.push_str(" ago");
            }
            rest = &rest[found + " ago".len()..];
        }
        out.push_str(rest);
        out
    }

    fn real_ago(secs: i64) -> String {
        let now = chrono::Utc::now();
        let at = (now - chrono::Duration::seconds(secs)).to_rfc3339();
        ralph_core::chat_thread_table::ago(&at, now).unwrap()
    }

    #[test]
    fn the_pattern_is_the_one_in_the_script() {
        let script = mask_script(Screen::Threads).unwrap();
        assert!(script.contains(&format!("/{AGO_PATTERN}/g")));
        assert!(!script.contains("AGO_PATTERN"));
        assert!(!script.contains("PLACEHOLDER"));
        assert!(script.contains(&format!("'{PLACEHOLDER}'")));
    }

    #[test]
    fn every_form_the_thread_table_prints_is_masked() {
        let samples = [
            (4, "4s"),
            (300, "5m"),
            (3 * 3600, "3h"),
            (269 * 86400, "269d"),
        ];
        for (secs, unit) in samples {
            assert_eq!(real_ago(secs), format!("{unit} ago"));
            assert_eq!(masked(&real_ago(secs)), PLACEHOLDER);
        }
        assert_eq!(
            masked("parked 4s ago and merged 269d ago"),
            format!("parked {PLACEHOLDER} and merged {PLACEHOLDER}")
        );
    }

    #[test]
    fn text_the_thread_table_never_prints_is_not_fully_masked() {
        assert_eq!(masked("just now"), "just now");
        assert_eq!(masked("ago"), "ago");
        assert_eq!(masked("1h 3m ago"), format!("1h {PLACEHOLDER}"));
    }

    #[test]
    fn screens_without_relative_times_have_no_mask() {
        for screen in [
            Screen::Memory,
            Screen::Workers,
            Screen::Schedules,
            Screen::Watches,
            Screen::Config,
            Screen::Catalogue,
        ] {
            assert_eq!(mask_script(screen), None);
        }
    }
}
