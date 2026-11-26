use super::core::Screen;

pub const PLACEHOLDER: &str = "--";

const AGO_MASK_TEMPLATE: &str = r"const walker = document.createTreeWalker(document.getElementById('main'), NodeFilter.SHOW_TEXT); const nodes = []; while (walker.nextNode()) nodes.push(walker.currentNode); nodes.forEach((node) => { node.nodeValue = node.nodeValue.replace(/\d+[a-z]+ ago/g, 'PLACEHOLDER'); }); return nodes.length;";

pub fn mask_script(screen: Screen) -> Option<String> {
    match screen {
        Screen::Threads => Some(AGO_MASK_TEMPLATE.replace("PLACEHOLDER", PLACEHOLDER)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threads_has_a_mask_that_rewrites_ago_text() {
        let script = mask_script(Screen::Threads).unwrap();
        assert!(script.contains(" ago"));
        assert!(script.contains(&format!("'{PLACEHOLDER}'")));
        assert!(script.contains("getElementById('main')"));
        assert!(script.contains("createTreeWalker"));
    }

    #[test]
    fn script_quotes_are_balanced_and_unescaped() {
        let script = mask_script(Screen::Threads).unwrap();
        assert_eq!(script.matches('\'').count() % 2, 0);
        assert!(!script.contains('"'));
        assert!(!script.contains('\\') || script.contains("\\d"));
    }

    #[test]
    fn placeholder_is_plain_and_short() {
        assert!(!PLACEHOLDER.is_empty());
        assert!(PLACEHOLDER.len() <= 3);
        assert!(PLACEHOLDER.chars().all(|c| c == '-'));
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
