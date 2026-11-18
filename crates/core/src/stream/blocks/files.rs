use std::path::Path;

use super::ImageFile;

pub trait Files {
    fn read_text(&self, path: &Path) -> Option<String>;
    fn exists(&self, path: &Path) -> bool;
    fn image(&self, path: &str) -> ImageFile;
}
