use ralph_core::diff_command::DiffPager;
use serde::{Deserialize, Serialize};

fn default_changes_pager() -> String {
    "git".to_string()
}

/// `[changes]` pager for `/diff`. An unknown `pager` string does not fail
/// parsing: [`ChangesSection::warnings`] reports it and [`ChangesSection::pager`]
/// falls back to git.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangesSection {
    #[serde(default = "default_changes_pager")]
    pager: String,
    #[serde(default)]
    pub diff_args: Vec<String>,
    #[serde(default = "super::default_true")]
    pub enabled: bool,
}

impl Default for ChangesSection {
    fn default() -> Self {
        Self {
            pager: default_changes_pager(),
            diff_args: Vec::new(),
            enabled: true,
        }
    }
}

impl ChangesSection {
    pub fn pager(&self) -> DiffPager {
        match self.pager.as_str() {
            "delta" => DiffPager::Delta,
            "hunk" => DiffPager::Hunk,
            _ => DiffPager::Git,
        }
    }

    pub fn warnings(&self) -> Vec<String> {
        match self.pager.as_str() {
            "git" | "delta" | "hunk" => Vec::new(),
            other => vec![format!(
                "[changes] pager = \"{other}\" is unknown; using git"
            )],
        }
    }
}
