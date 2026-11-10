use crate::markdown::{RolePalette, ThemeRole};
use crate::theme_slots::ResolveColors;
use ralph_server::config::ThemeSection;

mod check;
pub use check::{check, hard_errors, missing_config_dir};
mod command;
mod mermaid_style;
#[cfg(test)]
mod theme_tests;
mod view;
pub use command::{run_check, run_show};
pub use mermaid_style::MermaidStyleExt;
pub use view::{
    config_view, effective_now, env_settings, load_for_chat, ConfigSnapshot, FileState, Origin,
    Setting,
};

pub trait ThemeWarnings {
    fn warnings(&self) -> Vec<String>;
}

impl ThemeWarnings for ThemeSection {
    fn warnings(&self) -> Vec<String> {
        self.assistant
            .palette(ThemeRole::Assistant)
            .1
            .into_iter()
            .chain(self.agent().palette(ThemeRole::Agent).1)
            .chain(self.status.resolve().1)
            .chain(self.input.resolve().1)
            .chain(self.diff.resolve().1)
            .chain(self.tools.resolve().1)
            .chain(self.ui.resolve().1)
            .collect()
    }
}
