use crate::cli::StatsArgs;
use ralph_core::chat_memory::{build_stats, history_path, render_stats, timing_path, StatsConfig};
use std::io;
use std::path::Path;

fn read_lossy(path: &Path) -> io::Result<String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(String::from_utf8_lossy(&bytes).into_owned()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => Err(err),
    }
}

pub(super) fn print_stats(
    store_dir: &Path,
    args: &StatsArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = StatsConfig {
        gap_secs: args.gap,
        few_hits: args.few_hits,
        ..StatsConfig::default()
    };
    let history = read_lossy(&history_path(store_dir))?;
    let timing = read_lossy(&timing_path(store_dir))?;
    println!("{}", render_stats(&build_stats(&history, &timing, &config)));
    Ok(())
}
