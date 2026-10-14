use std::fmt;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
pub const MAX_TURN_BYTES: usize = 80 * 1024 * 1024;

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

    pub fn media_type(self) -> &'static str {
        match self {
            ImageKind::Png => "image/png",
            ImageKind::Jpeg => "image/jpeg",
            ImageKind::Gif => "image/gif",
            ImageKind::Webp => "image/webp",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum AttachmentError {
    NotAnImage,
    ImageTooLarge { bytes: usize },
    TurnTooLarge { bytes: usize },
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
            AttachmentError::TurnTooLarge { bytes } => {
                write!(f, "images total {bytes} bytes, limit is {MAX_TURN_BYTES}")
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

impl Image {
    pub fn kind(&self) -> ImageKind {
        self.kind
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

pub struct SavedImage<'a> {
    pub image: &'a Image,
    pub path: &'a str,
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

pub fn check_turn(images: &[Image]) -> Result<(), AttachmentError> {
    let total: usize = images.iter().map(|image| image.bytes.len()).sum();
    if total > MAX_TURN_BYTES {
        return Err(AttachmentError::TurnTooLarge { bytes: total });
    }
    Ok(())
}

pub fn file_name(image: &Image) -> String {
    let digest = Sha256::digest(&image.bytes);
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{hex}.{}", image.kind.extension())
}

pub fn placeholder(number: usize, path: &str) -> String {
    format!("[Image #{number}: {path}]")
}

pub fn user_content(images: &[SavedImage], text: &str) -> Value {
    let mut blocks: Vec<Value> = images
        .iter()
        .map(|saved| {
            json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": saved.image.kind.media_type(),
                    "data": STANDARD.encode(&saved.image.bytes),
                },
            })
        })
        .collect();
    let placeholders = images
        .iter()
        .enumerate()
        .map(|(index, saved)| placeholder(index + 1, saved.path))
        .collect::<Vec<_>>()
        .join("\n");
    let text = if images.is_empty() {
        text.to_string()
    } else {
        format!("{text}\n\n{placeholders}")
    };
    blocks.push(json!({"type": "text", "text": text}));
    Value::Array(blocks)
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
    fn turn_limit_boundary() {
        let image = parse_image(png_of_len(MAX_IMAGE_BYTES)).unwrap();
        let at_limit = vec![image.clone(); 8];
        assert_eq!(check_turn(&at_limit), Ok(()));
        let over = vec![image; 9];
        assert_eq!(
            check_turn(&over),
            Err(AttachmentError::TurnTooLarge {
                bytes: 9 * MAX_IMAGE_BYTES
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
    fn user_content_puts_images_first_then_text_with_placeholders() {
        let first = png();
        let second = parse_image(b"GIF89a".to_vec()).unwrap();
        let saved = [
            SavedImage {
                image: &first,
                path: "/a.png",
            },
            SavedImage {
                image: &second,
                path: "/b.gif",
            },
        ];
        let content = user_content(&saved, "look");
        assert_eq!(
            content,
            json!([
                {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "iVBORw0KGgo="}},
                {"type": "image", "source": {"type": "base64", "media_type": "image/gif", "data": "R0lGODlh"}},
                {"type": "text", "text": "look\n\n[Image #1: /a.png]\n[Image #2: /b.gif]"},
            ])
        );
    }

    #[test]
    fn user_content_without_images_keeps_text() {
        assert_eq!(
            user_content(&[], "hi"),
            json!([{"type": "text", "text": "hi"}])
        );
    }
}
