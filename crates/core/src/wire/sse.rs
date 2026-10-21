use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Debug, Default)]
pub struct SseDecoder {
    buffer: Vec<u8>,
    bom_checked: bool,
    event: Option<String>,
    data: String,
}

const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

impl SseDecoder {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buffer.extend_from_slice(chunk);
        if !self.bom_checked {
            if self.buffer.len() < BOM.len() && BOM.starts_with(&self.buffer) {
                return Vec::new();
            }
            self.bom_checked = true;
            if self.buffer.starts_with(&BOM) {
                self.buffer.drain(..BOM.len());
            }
        }
        let mut events = Vec::new();
        let mut start = 0;
        while let Some((end, next)) = next_line(&self.buffer, start) {
            let line = String::from_utf8_lossy(&self.buffer[start..end]).into_owned();
            start = next;
            if let Some(event) = self.line(&line) {
                events.push(event);
            }
        }
        self.buffer.drain(..start);
        events
    }

    fn line(&mut self, line: &str) -> Option<SseEvent> {
        if line.is_empty() {
            let event = self.event.take();
            if self.data.is_empty() {
                return None;
            }
            let mut data = std::mem::take(&mut self.data);
            data.pop();
            return Some(SseEvent { event, data });
        }
        if line.starts_with(':') {
            return None;
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => self.event = Some(value.to_string()),
            "data" => {
                self.data.push_str(value);
                self.data.push('\n');
            }
            _ => {}
        }
        None
    }
}

fn next_line(buffer: &[u8], start: usize) -> Option<(usize, usize)> {
    let offset = buffer[start..]
        .iter()
        .position(|&b| b == b'\n' || b == b'\r')?;
    let end = start + offset;
    if buffer[end] == b'\n' {
        return Some((end, end + 1));
    }
    match buffer.get(end + 1) {
        Some(b'\n') => Some((end, end + 2)),
        Some(_) => Some((end, end + 1)),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRANSCRIPT: &str = "event: message_start\ndata: {\"type\":\"message_start\"}\n\n: ping\n\nevent: content_block_delta\ndata: {\"text\":\"héllo — 日本 🎉\"}\n\nevent: message_delta\ndata: {\"usage\":1}\n\ndata: line one\ndata: line two\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";

    fn ev(event: Option<&str>, data: &str) -> SseEvent {
        SseEvent {
            event: event.map(String::from),
            data: data.to_string(),
        }
    }

    fn expected() -> Vec<SseEvent> {
        vec![
            ev(Some("message_start"), "{\"type\":\"message_start\"}"),
            ev(
                Some("content_block_delta"),
                "{\"text\":\"héllo — 日本 🎉\"}",
            ),
            ev(Some("message_delta"), "{\"usage\":1}"),
            ev(None, "line one\nline two"),
            ev(Some("message_stop"), "{\"type\":\"message_stop\"}"),
        ]
    }

    fn decode_chunks(chunks: &[&[u8]]) -> Vec<SseEvent> {
        let mut decoder = SseDecoder::default();
        chunks.iter().flat_map(|c| decoder.push(c)).collect()
    }

    fn check_all_splits(input: &[u8], expected: &[SseEvent]) {
        assert_eq!(decode_chunks(&[input]), expected);
        for i in 0..=input.len() {
            let (a, b) = input.split_at(i);
            assert_eq!(decode_chunks(&[a, b]), expected, "split at {i}");
        }
        let singles: Vec<&[u8]> = input.chunks(1).collect();
        assert_eq!(decode_chunks(&singles), expected);
    }

    #[test]
    fn lf_transcript() {
        check_all_splits(TRANSCRIPT.as_bytes(), &expected());
    }

    #[test]
    fn crlf_transcript() {
        let input = TRANSCRIPT.replace('\n', "\r\n");
        check_all_splits(input.as_bytes(), &expected());
    }

    #[test]
    fn lone_cr_transcript() {
        let input = format!("{}x\r", TRANSCRIPT.replace('\n', "\r"));
        check_all_splits(input.as_bytes(), &expected());
    }

    #[test]
    fn bom_split_is_stripped() {
        let mut input = BOM.to_vec();
        input.extend_from_slice(b"data: x\n\n");
        check_all_splits(&input, &[ev(None, "x")]);
    }

    #[test]
    fn empty_data_not_dispatched() {
        assert_eq!(decode_chunks(&[b"event: x\n\n\n"]), Vec::new());
    }

    #[test]
    fn unterminated_last_event_not_emitted() {
        assert_eq!(
            decode_chunks(&[b"data: a\n\ndata: b\n"]),
            vec![ev(None, "a")]
        );
    }
}
