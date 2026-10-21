use flate2::read::{DeflateDecoder, MultiGzDecoder, ZlibDecoder};
use std::fmt;
use std::io::Read;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    Unsupported(String),
    Corrupt { encoding: String, message: String },
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(coding) => write!(f, "unsupported content-encoding: {coding}"),
            Self::Corrupt { encoding, message } => {
                write!(f, "corrupt {encoding} body: {message}")
            }
        }
    }
}

impl std::error::Error for DecodeError {}

pub fn decode_body(content_encoding: Option<&str>, body: &[u8]) -> Result<Vec<u8>, DecodeError> {
    content_encoding
        .unwrap_or("")
        .split(',')
        .map(|coding| coding.trim().to_ascii_lowercase())
        .filter(|coding| !coding.is_empty())
        .rev()
        .try_fold(body.to_vec(), |bytes, coding| decode_one(&coding, &bytes))
}

fn decode_one(coding: &str, bytes: &[u8]) -> Result<Vec<u8>, DecodeError> {
    match coding {
        "identity" => Ok(bytes.to_vec()),
        "gzip" | "x-gzip" => read_all(coding, MultiGzDecoder::new(bytes)),
        "deflate" => read_all(coding, ZlibDecoder::new(bytes))
            .or_else(|_| read_all(coding, DeflateDecoder::new(bytes))),
        other => Err(DecodeError::Unsupported(other.to_string())),
    }
}

fn read_all(coding: &str, mut reader: impl Read) -> Result<Vec<u8>, DecodeError> {
    let mut out = Vec::new();
    reader
        .read_to_end(&mut out)
        .map_err(|e| DecodeError::Corrupt {
            encoding: coding.to_string(),
            message: e.to_string(),
        })?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
    use flate2::Compression;
    use std::io::Write;

    const TEXT: &[u8] = b"hello wire";

    fn gzip(data: &[u8]) -> Vec<u8> {
        let mut e = GzEncoder::new(Vec::new(), Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    #[test]
    fn identity_and_none() {
        assert_eq!(decode_body(None, TEXT).unwrap(), TEXT);
        assert_eq!(decode_body(Some("identity"), TEXT).unwrap(), TEXT);
        assert_eq!(decode_body(Some(""), TEXT).unwrap(), TEXT);
    }

    #[test]
    fn gzip_variants() {
        assert_eq!(decode_body(Some("gzip"), &gzip(TEXT)).unwrap(), TEXT);
        assert_eq!(decode_body(Some("x-gzip"), &gzip(TEXT)).unwrap(), TEXT);
        assert_eq!(decode_body(Some("  GZIP "), &gzip(TEXT)).unwrap(), TEXT);
    }

    #[test]
    fn deflate_zlib_and_raw() {
        let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
        z.write_all(TEXT).unwrap();
        assert_eq!(
            decode_body(Some("deflate"), &z.finish().unwrap()).unwrap(),
            TEXT
        );
        let mut r = DeflateEncoder::new(Vec::new(), Compression::default());
        r.write_all(TEXT).unwrap();
        assert_eq!(
            decode_body(Some("deflate"), &r.finish().unwrap()).unwrap(),
            TEXT
        );
    }

    #[test]
    fn chained_gzip() {
        let twice = gzip(&gzip(TEXT));
        assert_eq!(decode_body(Some("gzip, gzip"), &twice).unwrap(), TEXT);
    }

    #[test]
    fn unsupported_codings() {
        assert_eq!(
            decode_body(Some("br"), TEXT),
            Err(DecodeError::Unsupported("br".into()))
        );
        assert_eq!(
            decode_body(Some("zstd"), TEXT).unwrap_err().to_string(),
            "unsupported content-encoding: zstd"
        );
    }

    #[test]
    fn corrupt_gzip() {
        assert!(matches!(
            decode_body(Some("gzip"), b"not gzip data"),
            Err(DecodeError::Corrupt { .. })
        ));
    }
}
