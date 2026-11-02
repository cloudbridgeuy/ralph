const PUSH_TITLE: &str = "\x1b[22;0t";
const POP_TITLE: &str = "\x1b[23;0t";

pub fn title_start(name: &str) -> String {
    let clean: String = name.chars().filter(|c| !c.is_control()).collect();
    format!("{PUSH_TITLE}\x1b]2;ralph · {clean}\x07")
}

pub fn title_end() -> &'static str {
    POP_TITLE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_pushes_the_old_title_then_sets_the_new_one() {
        assert_eq!(title_start("review"), "\x1b[22;0t\x1b]2;ralph · review\x07");
    }

    #[test]
    fn start_strips_control_characters_from_the_name() {
        assert_eq!(
            title_start("a\x1b]0;x\x07b\n"),
            "\x1b[22;0t\x1b]2;ralph · a]0;xb\x07"
        );
    }

    #[test]
    fn end_pops_the_saved_title() {
        assert_eq!(title_end(), "\x1b[23;0t");
    }
}
