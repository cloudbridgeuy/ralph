use crate::wire::usage::{CacheTier, CacheWrite, Usage};

pub struct AgyUsage {
    pub total: u64,
    pub input: u64,
    pub output: u64,
    pub read: Option<u64>,
    pub write: Option<u64>,
    pub thought: Option<u64>,
}

pub fn agy_turn_usage(raw: &AgyUsage) -> Result<Option<Usage>, &'static str> {
    let read = raw.read.unwrap_or(0);
    let write = raw.write.unwrap_or(0);
    let thought = raw.thought.unwrap_or(0);
    if raw.total == 0
        && raw.input == 0
        && raw.output == 0
        && read == 0
        && write == 0
        && thought == 0
    {
        return Ok(None);
    }
    let mut candidates = Vec::new();
    let inclusive_input = raw
        .input
        .checked_sub(read)
        .and_then(|n| n.checked_sub(write));
    for input in [inclusive_input, Some(raw.input)].into_iter().flatten() {
        for output in [
            (thought <= raw.output).then_some(raw.output),
            raw.output.checked_add(thought),
        ]
        .into_iter()
        .flatten()
        {
            let total = input
                .checked_add(read)
                .and_then(|n| n.checked_add(write))
                .and_then(|n| n.checked_add(output));
            if total == Some(raw.total) && !candidates.contains(&(input, output)) {
                candidates.push((input, output));
            }
        }
    }
    let [(input, output)] = candidates.as_slice() else {
        return Err("ambiguous or inconsistent agy token usage");
    };
    Ok(Some(Usage {
        input_uncached: *input,
        cache_read: read,
        cache_writes: if write == 0 {
            vec![]
        } else {
            vec![CacheWrite {
                tier: CacheTier::Unspecified,
                tokens: write,
            }]
        },
        output: *output,
        reasoning: raw.thought,
        other: vec![],
    }))
}
