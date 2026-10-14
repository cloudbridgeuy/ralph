use std::fmt;

use sha2::{Digest, Sha256};

pub const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;

const PNG_MAGIC: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
const JPEG_MAGIC: [u8; 3] = [0xFF, 0xD8, 0xFF];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    Png,
    Jpeg,
    Gif,
    Webp,
}

impl ImageKind {
    pub fn extension(self) -> &'static str {
        match self {
            ImageKind::Png => "png",
            ImageKind::Jpeg => "jpg",
            ImageKind::Gif => "gif",
            ImageKind::Webp => "webp",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum AttachmentError {
    NotAnImage,
    ImageTooLarge { bytes: usize },
}

impl fmt::Display for AttachmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AttachmentError::NotAnImage => {
                write!(f, "not a PNG, JPEG, GIF or WebP image")
            }
            AttachmentError::ImageTooLarge { bytes } => {
                write!(f, "image is {bytes} bytes, limit is {MAX_IMAGE_BYTES}")
            }
        }
    }
}

impl std::error::Error for AttachmentError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    kind: ImageKind,
    bytes: Vec<u8>,
}

pub fn detect(bytes: &[u8]) -> Option<ImageKind> {
    if bytes.starts_with(&PNG_MAGIC) {
        Some(ImageKind::Png)
    } else if bytes.starts_with(&JPEG_MAGIC) {
        Some(ImageKind::Jpeg)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(ImageKind::Gif)
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP".as_slice()) {
        Some(ImageKind::Webp)
    } else {
        None
    }
}

pub fn parse_image(bytes: Vec<u8>) -> Result<Image, AttachmentError> {
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(AttachmentError::ImageTooLarge { bytes: bytes.len() });
    }
    let kind = detect(&bytes).ok_or(AttachmentError::NotAnImage)?;
    Ok(Image { kind, bytes })
}

pub fn file_name(image: &Image) -> String {
    let digest = Sha256::digest(&image.bytes);
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{hex}.{}", image.kind.extension())
}

pub fn placeholder(number: usize, path: &str) -> String {
    format!("[Image #{number}: {path}]")
}

pub fn next_number(draft: &str) -> usize {
    draft
        .split("[Image #")
        .skip(1)
        .filter_map(|rest| {
            let (digits, _) = rest.split_once(':')?;
            digits.parse::<usize>().ok()
        })
        .max()
        .map_or(1, |largest| largest + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: [u8; 8] = PNG_MAGIC;

    fn png_of_len(len: usize) -> Vec<u8> {
        let mut bytes = vec![0u8; len];
        bytes[..PNG.len()].copy_from_slice(&PNG);
        bytes
    }

    fn png() -> Image {
        parse_image(PNG.to_vec()).unwrap()
    }

    #[test]
    fn detects_each_magic() {
        assert_eq!(detect(&PNG), Some(ImageKind::Png));
        assert_eq!(detect(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(ImageKind::Jpeg));
        assert_eq!(detect(b"GIF87a.."), Some(ImageKind::Gif));
        assert_eq!(detect(b"GIF89a.."), Some(ImageKind::Gif));
        assert_eq!(detect(b"RIFF\0\0\0\0WEBPVP8 "), Some(ImageKind::Webp));
    }

    #[test]
    fn rejects_unknown_empty_and_truncated() {
        assert_eq!(detect(b"hello world"), None);
        assert_eq!(detect(&[]), None);
        assert_eq!(detect(b"RIFF\0\0\0\0WAVE"), None);
        assert_eq!(detect(b"RIFF"), None);
        assert_eq!(parse_image(Vec::new()), Err(AttachmentError::NotAnImage));
    }

    #[test]
    fn per_image_limit_boundary() {
        assert!(parse_image(png_of_len(MAX_IMAGE_BYTES)).is_ok());
        assert_eq!(
            parse_image(png_of_len(MAX_IMAGE_BYTES + 1)),
            Err(AttachmentError::ImageTooLarge {
                bytes: MAX_IMAGE_BYTES + 1
            })
        );
    }

    #[test]
    fn file_name_is_sha256_hex_with_extension() {
        assert_eq!(
            file_name(&png()),
            "4c4b6a3be1314ab86138bef4314dde022e600960d8689a2c8f8631802d20dab6.png"
        );
    }

    #[test]
    fn placeholder_format() {
        assert_eq!(placeholder(2, "/tmp/a.png"), "[Image #2: /tmp/a.png]");
    }

    #[test]
    fn next_number_follows_largest_placeholder() {
        assert_eq!(next_number(""), 1);
        assert_eq!(next_number("[Image #1: /a.png]"), 2);
        assert_eq!(next_number("[Image #1: /a] x [Image #3: /b]"), 4);
        assert_eq!(next_number("[Image #x: /a]"), 1);
    }
}
