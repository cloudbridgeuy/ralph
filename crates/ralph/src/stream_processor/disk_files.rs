use std::fs;
use std::path::Path;

use ralph_core::stream::blocks::Files;

pub struct DiskFiles;

impl Files for DiskFiles {
    fn read_text(&self, path: &Path) -> Option<String> {
        fs::read_to_string(path).ok()
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }
}
