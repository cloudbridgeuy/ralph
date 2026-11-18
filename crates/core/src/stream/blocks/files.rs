use std::path::Path;

pub trait Files {
    fn read_text(&self, path: &Path) -> Option<String>;
    fn exists(&self, path: &Path) -> bool;
}
