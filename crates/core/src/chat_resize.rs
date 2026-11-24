pub const DEBOUNCE_MS: u64 = 250;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResizeInput {
    pub rendered: Option<usize>,
    pub new_cols: usize,
    pub idle: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Render {
    Chunk(usize),
    Reprint(usize),
}

pub fn next_width(prev: Option<usize>, render: Render) -> usize {
    match render {
        Render::Chunk(cols) => prev.map_or(cols, |prev| prev.min(cols)),
        Render::Reprint(cols) => cols,
    }
}

pub fn should_redraw(input: &ResizeInput) -> bool {
    input.idle
        && input
            .rendered
            .is_some_and(|rendered| input.new_cols < rendered)
}

pub fn debounce(last_ms: Option<u64>, now_ms: u64, window_ms: u64) -> bool {
    last_ms.is_none_or(|last| now_ms.saturating_sub(last) >= window_ms)
}

#[cfg(test)]
mod tests;
