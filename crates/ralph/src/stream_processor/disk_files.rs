use std::fs;
use std::io::Read;
use std::path::Path;

use ralph_core::stream::blocks::{Files, ImageFile};

use crate::render::image::resolve;

const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;

pub struct DiskFiles;

fn read_capped(path: &Path) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(MAX_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= MAX_IMAGE_BYTES).then_some(bytes)
}

impl Files for DiskFiles {
    fn read_text(&self, path: &Path) -> Option<String> {
        fs::read_to_string(path).ok()
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn image(&self, path: &str) -> ImageFile {
        let resolved = resolve(path);
        let bytes = read_capped(&resolved);
        let shown = fs::canonicalize(&resolved)
            .or_else(|_| std::path::absolute(&resolved))
            .unwrap_or(resolved);
        ImageFile {
            path: path.to_string(),
            shown,
            bytes,
        }
    }
}
