use ralph_core::schedule::{parse_answer, Answer};
use std::io;
use std::path::Path;
use std::sync::Arc;

pub trait Ask {
    fn answer(&mut self, prompt: &str) -> Option<String>;
    fn edit(&mut self, path: &Path) -> io::Result<()>;
}

pub type Notices = Arc<dyn Fn(&str) + Send + Sync>;

pub fn yes_no(ask: &mut dyn Ask, question: impl Fn() -> String) -> Option<bool> {
    loop {
        let line = ask.answer(&question())?;
        match parse_answer(&line) {
            Answer::Yes => return Some(true),
            Answer::No => return Some(false),
            Answer::Ask => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scripted {
        lines: Vec<String>,
        prompts: Vec<String>,
    }

    impl Ask for Scripted {
        fn answer(&mut self, prompt: &str) -> Option<String> {
            self.prompts.push(prompt.to_string());
            self.lines.pop()
        }

        fn edit(&mut self, _path: &Path) -> io::Result<()> {
            Ok(())
        }
    }

    fn scripted(lines: &[&str]) -> Scripted {
        Scripted {
            lines: lines.iter().rev().map(|l| l.to_string()).collect(),
            prompts: Vec::new(),
        }
    }

    #[test]
    fn yes_no_repeats_unclear_and_none_on_closed() {
        let mut ask = scripted(&["maybe", "", "yes"]);
        assert_eq!(yes_no(&mut ask, || "go? ".to_string()), Some(true));
        assert_eq!(ask.prompts.len(), 3);
        assert_eq!(
            yes_no(&mut scripted(&["n"]), || "go? ".to_string()),
            Some(false)
        );
        assert_eq!(yes_no(&mut scripted(&[]), || "go? ".to_string()), None);
    }
}
