use super::query::tokenize;
use super::request::{check_cursor, check_fingerprint, passes, SearchCursor, SearchRequest};
use crate::chat_memory::archive::{Archive, ByteSource};
use crate::chat_memory::lookup::excerpt;
use crate::chat_memory::{Entry, Role};
use std::io;

pub(super) const CHUNK_ENTRIES: usize = 256;
pub(super) const SNIPPET_CHARS: usize = 160;
const SNIPPET_LEAD_CHARS: usize = 40;
const K1: f64 = 1.2;
const B: f64 = 0.75;

struct Candidate {
    id: usize,
    role: Role,
    at: String,
    counts: Vec<u32>,
    length: usize,
    snippet: String,
}

struct Scan<'a> {
    request: &'a SearchRequest,
    eligible: usize,
    total_length: usize,
    frequency: Vec<usize>,
    candidates: Vec<Candidate>,
}

impl<'a> Scan<'a> {
    fn new(request: &'a SearchRequest) -> Self {
        Scan {
            request,
            eligible: 0,
            total_length: 0,
            frequency: vec![0; request.query.units.len()],
            candidates: Vec::new(),
        }
    }

    fn push(&mut self, id: usize, entry: &Entry) {
        if entry.is_corrupt() || !passes(self.request, entry) {
            return;
        }
        let tokens = tokenize(&entry.text);
        self.eligible += 1;
        self.total_length += tokens.len();
        let found: Vec<(u32, Option<usize>)> = self
            .request
            .query
            .units
            .iter()
            .map(|unit| unit.occurrences(&tokens))
            .collect();
        for (seen, (count, _)) in self.frequency.iter_mut().zip(&found) {
            *seen += usize::from(*count > 0);
        }
        let units = &self.request.query.units;
        let required_met = units
            .iter()
            .zip(&found)
            .all(|(unit, (count, _))| !unit.required || *count > 0);
        let first = found.iter().filter_map(|(_, first)| *first).min();
        let Some(first) = first.filter(|_| required_met) else {
            return;
        };
        self.candidates.push(Candidate {
            id,
            role: entry.role,
            at: entry.at.clone(),
            counts: found.iter().map(|(count, _)| *count).collect(),
            length: tokens.len(),
            snippet: excerpt(&entry.text, first, SNIPPET_LEAD_CHARS, SNIPPET_CHARS),
        });
    }

    fn score(&self, candidate: &Candidate) -> f64 {
        let n = self.eligible as f64;
        let average = (self.total_length as f64 / n).max(1.0);
        let norm = K1 * (1.0 - B + B * candidate.length as f64 / average);
        candidate
            .counts
            .iter()
            .zip(&self.frequency)
            .map(|(count, df)| {
                let tf = f64::from(*count);
                let df = *df as f64;
                let idf = (1.0 + (n - df + 0.5) / (df + 0.5)).ln();
                idf * tf * (K1 + 1.0) / (tf + norm)
            })
            .sum()
    }

    fn ranked(self) -> Vec<Candidate> {
        let scores: Vec<f64> = self.candidates.iter().map(|c| self.score(c)).collect();
        let mut scored: Vec<(f64, Candidate)> = scores.into_iter().zip(self.candidates).collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.id.cmp(&a.1.id)));
        scored.into_iter().map(|(_, candidate)| candidate).collect()
    }
}

fn render(request: &SearchRequest, count: usize, ranked: &[Candidate]) -> String {
    if ranked.is_empty() {
        return format!("no hits for {}", request.text);
    }
    let offset = request.cursor.map_or(0, |c| c.offset).min(ranked.len());
    let end = (offset + request.limit).min(ranked.len());
    let mut lines: Vec<String> = ranked[offset..end]
        .iter()
        .map(|hit| {
            format!(
                "#{} · {} · {} · {}",
                hit.id,
                hit.role.as_str(),
                hit.at,
                hit.snippet
            )
        })
        .collect();
    lines.push(if end < ranked.len() {
        let next = SearchCursor {
            offset: end,
            count,
            fingerprint: request.fingerprint,
        };
        format!("next cursor: {}", next.encode())
    } else {
        "(end of results)".to_string()
    });
    lines.push("(read <id> for the full text)".to_string());
    lines.join("\n")
}

pub fn search<B: ByteSource>(
    archive: &mut Archive<B>,
    request: &SearchRequest,
) -> io::Result<Result<String, String>> {
    if let Err(message) = check_fingerprint(request) {
        return Ok(Err(message));
    }
    let mut scan = Scan::new(request);
    let count = archive.scan_entries(CHUNK_ENTRIES, |id, entry| scan.push(id, entry))?;
    if let Err(message) = check_cursor(request, count) {
        return Ok(Err(message));
    }
    Ok(Ok(render(request, count, &scan.ranked())))
}
