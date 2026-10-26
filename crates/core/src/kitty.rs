use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use std::time::Duration;

const APC_START: &str = "\x1b_G";
const APC_END: &str = "\x1b\\";
const CHUNK_BYTES: usize = 4096;
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
const ASSUMED_CELL_PX: (u32, u32) = (8, 16);
const PLACEHOLDER: char = '\u{10EEEE}';
const MAX_PLACEHOLDER_CELLS: u32 = 297;
const ROW_DIACRITICS: [char; 297] = [
    '\u{0305}',
    '\u{030d}',
    '\u{030e}',
    '\u{0310}',
    '\u{0312}',
    '\u{033d}',
    '\u{033e}',
    '\u{033f}',
    '\u{0346}',
    '\u{034a}',
    '\u{034b}',
    '\u{034c}',
    '\u{0350}',
    '\u{0351}',
    '\u{0352}',
    '\u{0357}',
    '\u{035b}',
    '\u{0363}',
    '\u{0364}',
    '\u{0365}',
    '\u{0366}',
    '\u{0367}',
    '\u{0368}',
    '\u{0369}',
    '\u{036a}',
    '\u{036b}',
    '\u{036c}',
    '\u{036d}',
    '\u{036e}',
    '\u{036f}',
    '\u{0483}',
    '\u{0484}',
    '\u{0485}',
    '\u{0486}',
    '\u{0487}',
    '\u{0592}',
    '\u{0593}',
    '\u{0594}',
    '\u{0595}',
    '\u{0597}',
    '\u{0598}',
    '\u{0599}',
    '\u{059c}',
    '\u{059d}',
    '\u{059e}',
    '\u{059f}',
    '\u{05a0}',
    '\u{05a1}',
    '\u{05a8}',
    '\u{05a9}',
    '\u{05ab}',
    '\u{05ac}',
    '\u{05af}',
    '\u{05c4}',
    '\u{0610}',
    '\u{0611}',
    '\u{0612}',
    '\u{0613}',
    '\u{0614}',
    '\u{0615}',
    '\u{0616}',
    '\u{0617}',
    '\u{0657}',
    '\u{0658}',
    '\u{0659}',
    '\u{065a}',
    '\u{065b}',
    '\u{065d}',
    '\u{065e}',
    '\u{06d6}',
    '\u{06d7}',
    '\u{06d8}',
    '\u{06d9}',
    '\u{06da}',
    '\u{06db}',
    '\u{06dc}',
    '\u{06df}',
    '\u{06e0}',
    '\u{06e1}',
    '\u{06e2}',
    '\u{06e4}',
    '\u{06e7}',
    '\u{06e8}',
    '\u{06eb}',
    '\u{06ec}',
    '\u{0730}',
    '\u{0732}',
    '\u{0733}',
    '\u{0735}',
    '\u{0736}',
    '\u{073a}',
    '\u{073d}',
    '\u{073f}',
    '\u{0740}',
    '\u{0741}',
    '\u{0743}',
    '\u{0745}',
    '\u{0747}',
    '\u{0749}',
    '\u{074a}',
    '\u{07eb}',
    '\u{07ec}',
    '\u{07ed}',
    '\u{07ee}',
    '\u{07ef}',
    '\u{07f0}',
    '\u{07f1}',
    '\u{07f3}',
    '\u{0816}',
    '\u{0817}',
    '\u{0818}',
    '\u{0819}',
    '\u{081b}',
    '\u{081c}',
    '\u{081d}',
    '\u{081e}',
    '\u{081f}',
    '\u{0820}',
    '\u{0821}',
    '\u{0822}',
    '\u{0823}',
    '\u{0825}',
    '\u{0826}',
    '\u{0827}',
    '\u{0829}',
    '\u{082a}',
    '\u{082b}',
    '\u{082c}',
    '\u{082d}',
    '\u{0951}',
    '\u{0953}',
    '\u{0954}',
    '\u{0f82}',
    '\u{0f83}',
    '\u{0f86}',
    '\u{0f87}',
    '\u{135d}',
    '\u{135e}',
    '\u{135f}',
    '\u{17dd}',
    '\u{193a}',
    '\u{1a17}',
    '\u{1a75}',
    '\u{1a76}',
    '\u{1a77}',
    '\u{1a78}',
    '\u{1a79}',
    '\u{1a7a}',
    '\u{1a7b}',
    '\u{1a7c}',
    '\u{1b6b}',
    '\u{1b6d}',
    '\u{1b6e}',
    '\u{1b6f}',
    '\u{1b70}',
    '\u{1b71}',
    '\u{1b72}',
    '\u{1b73}',
    '\u{1cd0}',
    '\u{1cd1}',
    '\u{1cd2}',
    '\u{1cda}',
    '\u{1cdb}',
    '\u{1ce0}',
    '\u{1dc0}',
    '\u{1dc1}',
    '\u{1dc3}',
    '\u{1dc4}',
    '\u{1dc5}',
    '\u{1dc6}',
    '\u{1dc7}',
    '\u{1dc8}',
    '\u{1dc9}',
    '\u{1dcb}',
    '\u{1dcc}',
    '\u{1dd1}',
    '\u{1dd2}',
    '\u{1dd3}',
    '\u{1dd4}',
    '\u{1dd5}',
    '\u{1dd6}',
    '\u{1dd7}',
    '\u{1dd8}',
    '\u{1dd9}',
    '\u{1dda}',
    '\u{1ddb}',
    '\u{1ddc}',
    '\u{1ddd}',
    '\u{1dde}',
    '\u{1ddf}',
    '\u{1de0}',
    '\u{1de1}',
    '\u{1de2}',
    '\u{1de3}',
    '\u{1de4}',
    '\u{1de5}',
    '\u{1de6}',
    '\u{1dfe}',
    '\u{20d0}',
    '\u{20d1}',
    '\u{20d4}',
    '\u{20d5}',
    '\u{20d6}',
    '\u{20d7}',
    '\u{20db}',
    '\u{20dc}',
    '\u{20e1}',
    '\u{20e7}',
    '\u{20e9}',
    '\u{20f0}',
    '\u{2cef}',
    '\u{2cf0}',
    '\u{2cf1}',
    '\u{2de0}',
    '\u{2de1}',
    '\u{2de2}',
    '\u{2de3}',
    '\u{2de4}',
    '\u{2de5}',
    '\u{2de6}',
    '\u{2de7}',
    '\u{2de8}',
    '\u{2de9}',
    '\u{2dea}',
    '\u{2deb}',
    '\u{2dec}',
    '\u{2dEd}',
    '\u{2dee}',
    '\u{2def}',
    '\u{2df0}',
    '\u{2df1}',
    '\u{2df2}',
    '\u{2df3}',
    '\u{2df4}',
    '\u{2df5}',
    '\u{2df6}',
    '\u{2df7}',
    '\u{2df8}',
    '\u{2df9}',
    '\u{2dfa}',
    '\u{2dfb}',
    '\u{2dfc}',
    '\u{2dfd}',
    '\u{2dfe}',
    '\u{2dff}',
    '\u{a66f}',
    '\u{a67c}',
    '\u{a67d}',
    '\u{a6f0}',
    '\u{a6f1}',
    '\u{a8e0}',
    '\u{a8e1}',
    '\u{a8e2}',
    '\u{a8e3}',
    '\u{a8e4}',
    '\u{a8e5}',
    '\u{a8e6}',
    '\u{a8e7}',
    '\u{a8e8}',
    '\u{a8e9}',
    '\u{a8ea}',
    '\u{a8eb}',
    '\u{a8ec}',
    '\u{a8ed}',
    '\u{a8ee}',
    '\u{a8ef}',
    '\u{a8f0}',
    '\u{a8f1}',
    '\u{aab0}',
    '\u{aab2}',
    '\u{aab3}',
    '\u{aab7}',
    '\u{aab8}',
    '\u{aabe}',
    '\u{aabf}',
    '\u{aac1}',
    '\u{fe20}',
    '\u{fe21}',
    '\u{fe22}',
    '\u{fe23}',
    '\u{fe24}',
    '\u{fe25}',
    '\u{fe26}',
    '\u{10a0f}',
    '\u{10a38}',
    '\u{1d185}',
    '\u{1d186}',
    '\u{1d187}',
    '\u{1d188}',
    '\u{1d189}',
    '\u{1d1aa}',
    '\u{1d1ab}',
    '\u{1d1ac}',
    '\u{1d1ad}',
    '\u{1d242}',
    '\u{1d243}',
    '\u{1d244}',
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Direct,
    Tmux,
}

pub fn tmux_wrap(seq: &str) -> String {
    format!("\x1bPtmux;{}\x1b\\", seq.replace('\x1b', "\x1b\x1b"))
}

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
    pub stdin_tty: bool,
    pub stdout_tty: bool,
    pub tmux: bool,
    pub passthrough: Option<String>,
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

fn encode_chunks(png: &[u8], first_keys: &str, later_keys: &str) -> Vec<String> {
    let encoded = STANDARD.encode(png);
    let chunks: Vec<&[u8]> = encoded.as_bytes().chunks(CHUNK_BYTES).collect();
    let last = chunks.len().saturating_sub(1);
    chunks
        .iter()
        .enumerate()
        .map(|(index, chunk)| {
            let more = u8::from(index < last);
            let payload = String::from_utf8_lossy(chunk);
            let keys = if index == 0 { first_keys } else { later_keys };
            format!("{APC_START}{keys}m={more};{payload}{APC_END}")
        })
        .collect()
}

pub fn encode_apc(png: &[u8], cols: u32, rows: u32) -> String {
    encode_chunks(png, &format!("a=T,f=100,c={cols},r={rows},C=1,q=2,"), "").concat()
}

pub fn encode_virtual(png: &[u8], id: u8, cols: u32, rows: u32) -> String {
    encode_chunks(
        png,
        &format!("a=T,U=1,i={id},f=100,c={cols},r={rows},q=2,"),
        &format!("i={id},"),
    )
    .iter()
    .map(|chunk| tmux_wrap(chunk))
    .collect()
}

pub fn placeholder_rows(id: u8, cols: u32, rows: u32) -> String {
    let extra = (1..cols).map(|_| PLACEHOLDER).collect::<String>();
    (0..rows as usize)
        .map(|row| {
            let diacritic = ROW_DIACRITICS[row.min(ROW_DIACRITICS.len() - 1)];
            format!(
                "\x1b[38;5;{id}m{PLACEHOLDER}{diacritic}{}{extra}\x1b[39m\n",
                ROW_DIACRITICS[0]
            )
        })
        .collect()
}

pub fn clamp_placeholder_grid((cols, rows): (u32, u32)) -> (u32, u32) {
    (
        cols.min(MAX_PLACEHOLDER_CELLS),
        rows.min(MAX_PLACEHOLDER_CELLS),
    )
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

const GRAPHICS_PROBE: &str = "\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\";
pub const GRAPHICS_QUERY_TIMEOUT: Duration = Duration::from_millis(200);

const GRAPHICS_OK: &[u8] = b"\x1b_Gi=31;OK\x1b\\";
const DA_START: &[u8] = b"\x1b[?";
const STATUS_OK: &[u8] = b"\x1b[0n";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphicsReply {
    Pending,
    Supported,
    Unsupported,
}

pub fn graphics_query(placement: Placement) -> String {
    match placement {
        Placement::Direct => format!("{GRAPHICS_PROBE}\x1b[c"),
        Placement::Tmux => tmux_wrap(&format!("{GRAPHICS_PROBE}\x1b[5n")),
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn reply_end(bytes: &[u8], placement: Placement) -> Option<usize> {
    match placement {
        Placement::Direct => {
            let start = find(bytes, DA_START)?;
            bytes[start..].contains(&b'c').then_some(start)
        }
        Placement::Tmux => find(bytes, STATUS_OK),
    }
}

pub fn graphics_reply(bytes: &[u8], placement: Placement) -> GraphicsReply {
    let Some(end) = reply_end(bytes, placement) else {
        return GraphicsReply::Pending;
    };
    if find(&bytes[..end], GRAPHICS_OK).is_some() {
        GraphicsReply::Supported
    } else {
        GraphicsReply::Unsupported
    }
}

pub fn graphics_probe(snapshot: &TerminalSnapshot) -> Option<Placement> {
    if !(snapshot.stdin_tty && snapshot.stdout_tty) {
        return None;
    }
    if !snapshot.tmux {
        return Some(Placement::Direct);
    }
    matches!(
        snapshot.passthrough.as_deref().map(str::trim),
        Some("on" | "all")
    )
    .then_some(Placement::Tmux)
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

    const OK: &str = "\x1b_Gi=31;OK\x1b\\";
    const DA: &str = "\x1b[?62;4c";

    fn reply(text: &str) -> GraphicsReply {
        graphics_reply(text.as_bytes(), Placement::Direct)
    }

    fn tmux_reply(text: &str) -> GraphicsReply {
        graphics_reply(text.as_bytes(), Placement::Tmux)
    }

    #[test]
    fn graphics_reply_decides_only_at_the_device_attributes_end() {
        assert_eq!(reply(&format!("{OK}{DA}")), GraphicsReply::Supported);
        assert_eq!(
            reply(&format!("\x1b_Gi=31;ENOENT:bad\x1b\\{DA}")),
            GraphicsReply::Unsupported
        );
        assert_eq!(reply(DA), GraphicsReply::Unsupported);
        assert_eq!(reply(OK), GraphicsReply::Pending);
        assert_eq!(reply(""), GraphicsReply::Pending);
    }

    #[test]
    fn graphics_reply_handles_split_and_typed_bytes() {
        let full = format!("{OK}{DA}");
        assert_eq!(reply(&full[..full.len() - 1]), GraphicsReply::Pending);
        assert_eq!(reply(&full), GraphicsReply::Supported);
        assert_eq!(reply(&format!("ab{OK}{DA}")), GraphicsReply::Supported);
    }

    #[test]
    fn tmux_reply_decides_at_the_status_report() {
        assert_eq!(
            tmux_reply(&format!("{OK}\x1b[0n")),
            GraphicsReply::Supported
        );
        assert_eq!(tmux_reply("\x1b[0n"), GraphicsReply::Unsupported);
        assert_eq!(tmux_reply(OK), GraphicsReply::Pending);
        assert_eq!(tmux_reply(&format!("{DA}{OK}")), GraphicsReply::Pending);
        assert_eq!(
            tmux_reply(&format!("{DA}{OK}\x1b[0n")),
            GraphicsReply::Supported
        );
    }

    #[test]
    fn queries_match_placement() {
        assert!(graphics_query(Placement::Direct).ends_with("\x1b[c"));
        let tmux = graphics_query(Placement::Tmux);
        assert!(tmux.starts_with("\x1bPtmux;\x1b\x1b_G"));
        assert!(tmux.ends_with("\x1b\x1b[5n\x1b\\"));
        assert!(!tmux.contains("[c"));
    }

    #[test]
    fn tmux_wrap_doubles_every_escape() {
        assert_eq!(
            tmux_wrap("a\x1bb\x1b\\"),
            "\x1bPtmux;a\x1b\x1bb\x1b\x1b\\\x1b\\"
        );
    }

    #[test]
    fn graphics_probe_decision_table() {
        let snap = |stdin_tty, stdout_tty, tmux, passthrough: Option<&str>| TerminalSnapshot {
            stdin_tty,
            stdout_tty,
            tmux,
            passthrough: passthrough.map(str::to_string),
        };
        let cases = [
            (snap(true, true, false, None), Some(Placement::Direct)),
            (snap(false, true, false, None), None),
            (snap(true, false, false, None), None),
            (snap(true, true, true, Some("on\n")), Some(Placement::Tmux)),
            (snap(true, true, true, Some("all")), Some(Placement::Tmux)),
            (snap(true, true, true, Some("off")), None),
            (snap(true, true, true, Some("")), None),
            (snap(true, true, true, None), None),
            (snap(false, true, true, Some("on")), None),
        ];
        for (snapshot, expected) in cases {
            assert_eq!(graphics_probe(&snapshot), expected, "{snapshot:?}");
        }
    }

    #[test]
    fn virtual_encoding_wraps_each_chunk_and_repeats_the_id() {
        let single = encode_virtual(b"abc", 7, 3, 2);
        assert_eq!(
            single,
            "\x1bPtmux;\x1b\x1b_Ga=T,U=1,i=7,f=100,c=3,r=2,q=2,m=0;YWJj\x1b\x1b\\\x1b\\"
        );
        let multi = encode_virtual(&[7u8; 3073], 7, 3, 2);
        let parts: Vec<&str> = multi.split_inclusive("\x1b\x1b\\\x1b\\").collect();
        assert_eq!(parts.len(), 2);
        assert!(parts[0].contains("U=1,i=7,f=100,c=3,r=2,q=2,m=1;"));
        assert!(parts[1].starts_with("\x1bPtmux;\x1b\x1b_Gi=7,m=0;"));
    }

    #[test]
    fn placeholder_rows_have_exact_bytes() {
        let row0 = "\x1b[38;5;7m\u{10EEEE}\u{305}\u{305}\u{10EEEE}\x1b[39m\n";
        let row1 = "\x1b[38;5;7m\u{10EEEE}\u{30D}\u{305}\u{10EEEE}\x1b[39m\n";
        assert_eq!(placeholder_rows(7, 2, 2), format!("{row0}{row1}"));
        assert_eq!(placeholder_rows(7, 4, 5).matches('\n').count(), 5);
    }

    #[test]
    fn placeholder_grid_is_clamped_to_the_diacritic_table() {
        assert_eq!(clamp_placeholder_grid((400, 500)), (297, 297));
        assert_eq!(clamp_placeholder_grid((80, 10)), (80, 10));
        assert_eq!(ROW_DIACRITICS.len(), MAX_PLACEHOLDER_CELLS as usize);
    }
}
