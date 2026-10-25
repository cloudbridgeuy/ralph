use base64::engine::general_purpose::STANDARD;
use base64::Engine;

const APC_START: &str = "\x1b_G";
const APC_END: &str = "\x1b\\";
const CHUNK_BYTES: usize = 4096;
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
const ASSUMED_CELL_PX: (u32, u32) = (8, 16);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Text(String),
    Image { alt: String, path: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    Text(String),
    Apc(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TerminalSnapshot {
    pub term_program: String,
    pub term: String,
    pub kitty_window_id: bool,
    pub tmux: bool,
    pub stdout_tty: bool,
}

#[derive(Default)]
struct FenceState(Option<(char, usize)>);

impl FenceState {
    fn is_open(&self) -> bool {
        self.0.is_some()
    }

    fn feed(&mut self, line: &str) {
        let trimmed = line.trim_start();
        let Some(marker) = trimmed.chars().next().filter(|c| matches!(c, '`' | '~')) else {
            return;
        };
        let run = trimmed.chars().take_while(|c| *c == marker).count();
        if run < 3 {
            return;
        }
        match self.0 {
            None => self.0 = Some((marker, run)),
            Some((open, len)) if open == marker && run >= len => {
                let rest = &trimmed[run..];
                if rest.trim().is_empty() {
                    self.0 = None;
                }
            }
            Some(_) => {}
        }
    }
}

fn is_remote(path: &str) -> bool {
    let Some((scheme, _)) = path.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
}

fn skip_code_span(line: &str, start: usize) -> usize {
    let bytes = line.as_bytes();
    let run = bytes[start..].iter().take_while(|b| **b == b'`').count();
    let mut i = start + run;
    while i < bytes.len() {
        if bytes[i] != b'`' {
            i += 1;
            continue;
        }
        let close = bytes[i..].iter().take_while(|b| **b == b'`').count();
        if close == run {
            return i + close;
        }
        i += close;
    }
    start + run
}

fn image_at(line: &str, start: usize) -> Option<(String, String, usize)> {
    let rest = line[start..].strip_prefix("![")?;
    let alt_end = rest.find(']')?;
    let alt = &rest[..alt_end];
    let target = rest[alt_end + 1..].strip_prefix('(')?;
    let path_end = target.find(')')?;
    let path = target[..path_end].trim();
    if path.is_empty() || is_remote(path) {
        return None;
    }
    let consumed = 2 + alt_end + 2 + path_end + 1;
    Some((alt.to_string(), path.to_string(), start + consumed))
}

pub fn parse_image_markers(text: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut buffer = String::new();
    let mut fence = FenceState::default();
    for line in text.split_inclusive('\n') {
        fence.feed(line);
        let fenced_line = fence.is_open() || line.trim_start().starts_with("```");
        if fenced_line {
            buffer.push_str(line);
            continue;
        }
        let bytes = line.as_bytes();
        let mut i = 0;
        let mut literal_from = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'`' => i = skip_code_span(line, i),
                b'!' => match image_at(line, i) {
                    Some((alt, path, end)) => {
                        buffer.push_str(&line[literal_from..i]);
                        if !buffer.is_empty() {
                            segments.push(Segment::Text(std::mem::take(&mut buffer)));
                        }
                        segments.push(Segment::Image { alt, path });
                        i = end;
                        literal_from = end;
                    }
                    None => i += 1,
                },
                _ => i += 1,
            }
        }
        buffer.push_str(&line[literal_from..]);
    }
    if !buffer.is_empty() {
        segments.push(Segment::Text(buffer));
    }
    segments
}

pub fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || bytes[..8] != PNG_SIGNATURE || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    (width > 0 && height > 0).then_some((width, height))
}

pub fn cell_grid(
    png: (u32, u32),
    cell_px: Option<(u32, u32)>,
    wrap_cols: u32,
    max_rows: u32,
) -> (u32, u32) {
    let (cell_w, cell_h) = cell_px
        .filter(|(w, h)| *w > 0 && *h > 0)
        .unwrap_or(ASSUMED_CELL_PX);
    let (cell_w, cell_h) = (u128::from(cell_w), u128::from(cell_h));
    let (width, height) = (u128::from(png.0), u128::from(png.1));
    let natural = width.div_ceil(cell_w);
    let cols = natural.min(u128::from(wrap_cols)).max(1);
    let rows = (cols * cell_w * height).div_ceil(width * cell_h).max(1);
    let max_rows = u128::from(max_rows.max(1));
    let (cols, rows) = if rows > max_rows {
        let fit = (max_rows * cell_h * width) / (height * cell_w);
        (fit.clamp(1, cols), max_rows)
    } else {
        (cols, rows)
    };
    (
        u32::try_from(cols).unwrap_or(u32::MAX),
        u32::try_from(rows).unwrap_or(u32::MAX),
    )
}

pub fn encode_apc(png: &[u8], cols: u32, rows: u32) -> String {
    let encoded = STANDARD.encode(png);
    let chunks: Vec<&[u8]> = encoded.as_bytes().chunks(CHUNK_BYTES).collect();
    let last = chunks.len().saturating_sub(1);
    chunks
        .iter()
        .enumerate()
        .map(|(index, chunk)| {
            let more = u8::from(index < last);
            let payload = String::from_utf8_lossy(chunk);
            if index == 0 {
                format!(
                    "{APC_START}a=T,f=100,c={cols},r={rows},C=1,q=2,m={more};{payload}{APC_END}"
                )
            } else {
                format!("{APC_START}m={more};{payload}{APC_END}")
            }
        })
        .collect()
}

pub fn split_apc(text: &str) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(APC_START) {
        if start > 0 {
            pieces.push(Piece::Text(rest[..start].to_string()));
        }
        let body = &rest[start..];
        let Some(end) = body[APC_START.len()..].find(APC_END) else {
            return pieces;
        };
        let stop = APC_START.len() + end + APC_END.len();
        pieces.push(Piece::Apc(body[..stop].to_string()));
        rest = &body[stop..];
    }
    if !rest.is_empty() {
        pieces.push(Piece::Text(rest.to_string()));
    }
    pieces
}

pub fn strip_kitty_apc(text: &str) -> String {
    split_apc(text)
        .into_iter()
        .filter_map(|piece| match piece {
            Piece::Text(text) => Some(text),
            Piece::Apc(_) => None,
        })
        .collect()
}

pub fn supports_images(snapshot: &TerminalSnapshot) -> bool {
    let kitty_like = snapshot.term_program == "ghostty"
        || snapshot.term_program == "rex"
        || snapshot.term == "xterm-ghostty"
        || snapshot.term == "xterm-kitty"
        || snapshot.kitty_window_id;
    snapshot.stdout_tty && !snapshot.tmux && kitty_like
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(alt: &str, path: &str) -> Segment {
        Segment::Image {
            alt: alt.into(),
            path: path.into(),
        }
    }

    fn text(value: &str) -> Segment {
        Segment::Text(value.into())
    }

    #[test]
    fn marker_splits_text_around_a_local_image() {
        assert_eq!(
            parse_image_markers("see ![chart](~/a.png) now\n"),
            vec![text("see "), image("chart", "~/a.png"), text(" now\n")]
        );
        assert_eq!(
            parse_image_markers("![](rel/a.png)"),
            vec![image("", "rel/a.png")]
        );
    }

    #[test]
    fn urls_stay_text() {
        for src in [
            "![a](https://x.io/a.png)",
            "![a](http://x.io/a.png)",
            "![a](data:image/png;base64,AAAA)",
            "![a](file:///tmp/a.png)",
        ] {
            assert_eq!(parse_image_markers(src), vec![text(src)], "{src}");
        }
    }

    #[test]
    fn fenced_and_inline_code_are_ignored() {
        let fenced = "```md\n![a](/tmp/a.png)\n```\n~~~\n![b](/tmp/b.png)\n~~~\n";
        assert_eq!(parse_image_markers(fenced), vec![text(fenced)]);
        let inline = "use `![a](/tmp/a.png)` and ``![b](/tmp/b.png)``";
        assert_eq!(parse_image_markers(inline), vec![text(inline)]);
        assert_eq!(
            parse_image_markers("```\nx\n```\n![a](/a.png)"),
            vec![text("```\nx\n```\n"), image("a", "/a.png")]
        );
    }

    fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = PNG_SIGNATURE.to_vec();
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes
    }

    #[test]
    fn png_size_reads_ihdr() {
        assert_eq!(png_size(&png_header(640, 480)), Some((640, 480)));
        assert_eq!(png_size(&png_header(0, 480)), None);
        assert_eq!(png_size(&png_header(1, 1)[..20]), None);
        assert_eq!(png_size(b"GIF89a................................"), None);
    }

    #[test]
    fn grid_uses_cell_size_and_caps_at_wrap() {
        assert_eq!(cell_grid((160, 160), Some((8, 16)), 80, 50), (20, 10));
        assert_eq!(cell_grid((1600, 800), Some((10, 20)), 40, 50), (40, 10));
        assert_eq!(cell_grid((1, 1), Some((10, 20)), 40, 50), (1, 1));
        assert_eq!(cell_grid((160, 160), None, 80, 50), (20, 10));
        assert_eq!(cell_grid((160, 160), Some((0, 0)), 80, 50), (20, 10));
        assert_eq!(cell_grid((160, 160), None, 0, 50), (1, 1));
    }

    #[test]
    fn grid_clamps_rows_to_max_and_keeps_aspect() {
        assert_eq!(cell_grid((1600, 3200), Some((8, 16)), 200, 40), (40, 40));
        assert_eq!(cell_grid((160, 160), Some((8, 16)), 80, 10), (20, 10));
        assert_eq!(cell_grid((160, 160), Some((8, 16)), 80, 5), (10, 5));
        assert_eq!(cell_grid((1, 1_000_000), None, 80, 50), (1, 50));
        assert_eq!(cell_grid((160, 160), Some((8, 16)), 80, 0), (2, 1));
    }

    fn payload_len(apc: &str) -> usize {
        apc.split_once(';').map_or(0, |(_, rest)| {
            rest.strip_suffix(APC_END).map_or(0, str::len)
        })
    }

    #[test]
    fn single_chunk_carries_all_keys_and_finishes() {
        let apc = encode_apc(b"abc", 3, 2);
        assert_eq!(apc, "\x1b_Ga=T,f=100,c=3,r=2,C=1,q=2,m=0;YWJj\x1b\\");
    }

    #[test]
    fn chunk_boundary_at_4096_stays_single() {
        let apc = encode_apc(&[7u8; 3072], 1, 1);
        assert_eq!(apc.matches(APC_START).count(), 1);
        assert_eq!(payload_len(&apc), 4096);
        assert!(apc.contains("m=0;"));
    }

    #[test]
    fn multi_chunk_puts_keys_only_on_first() {
        let apc = encode_apc(&[7u8; 3073], 5, 4);
        let parts: Vec<&str> = apc.split_inclusive(APC_END).collect();
        assert_eq!(parts.len(), 2);
        assert!(parts[0].starts_with("\x1b_Ga=T,f=100,c=5,r=4,C=1,q=2,m=1;"));
        assert_eq!(payload_len(parts[0]), 4096);
        assert!(parts[1].starts_with("\x1b_Gm=0;"));
        assert!(!parts[1].contains("a=T"));
        let long = encode_apc(&[7u8; 9000], 1, 1);
        let marks: Vec<&str> = long
            .split_inclusive(APC_END)
            .map(|p| &p[APC_START.len()..p.find(';').unwrap_or(0)])
            .collect();
        assert_eq!(marks.len(), 3);
        assert!(marks[0].ends_with("m=1") && marks[1] == "m=1" && marks[2] == "m=0");
    }

    #[test]
    fn strip_removes_terminated_and_trailing_unterminated() {
        let text = "a\x1b_Gm=0;QQ==\x1b\\b\x1b_Gm=1;cut";
        assert_eq!(strip_kitty_apc(text), "ab");
        assert_eq!(strip_kitty_apc("plain"), "plain");
    }

    #[test]
    fn split_separates_text_from_apc_spans() {
        assert_eq!(
            split_apc("x\x1b_Gm=0;QQ==\x1b\\y"),
            vec![
                Piece::Text("x".into()),
                Piece::Apc("\x1b_Gm=0;QQ==\x1b\\".into()),
                Piece::Text("y".into())
            ]
        );
    }

    #[test]
    fn supports_images_table() {
        let base = TerminalSnapshot {
            stdout_tty: true,
            ..TerminalSnapshot::default()
        };
        let with = |f: fn(&mut TerminalSnapshot)| {
            let mut s = base.clone();
            f(&mut s);
            supports_images(&s)
        };
        assert!(!supports_images(&base));
        assert!(with(|s| s.term_program = "ghostty".into()));
        assert!(with(|s| {
            s.term_program = "rex".into();
            s.term = "xterm-256color".into();
        }));
        assert!(with(|s| s.term = "xterm-ghostty".into()));
        assert!(with(|s| s.term = "xterm-kitty".into()));
        assert!(with(|s| s.kitty_window_id = true));
        assert!(!with(|s| {
            s.kitty_window_id = true;
            s.tmux = true;
        }));
        assert!(!with(|s| {
            s.kitty_window_id = true;
            s.stdout_tty = false;
        }));
        assert!(!with(|s| s.term = "xterm-256color".into()));
    }
}
