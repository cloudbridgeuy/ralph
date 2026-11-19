use crate::cli::DecayArgs;
use ralph_core::chat_memory::{
    build_decay, decode_rows, parse_log, parse_tree, render_decay, DecayConfig, TAGS_FILE,
};
use std::io;
use std::path::Path;

const LOG_FILE: &str = "log.jsonl";
const TREE_FILE: &str = "tree.jsonl";

fn read_or_empty(path: &Path) -> io::Result<Vec<u8>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(bytes),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(err),
    }
}

pub(super) fn print_decay(
    store_dir: &Path,
    args: &DecayArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let log = read_or_empty(&store_dir.join(LOG_FILE))?;
    let tree = read_or_empty(&store_dir.join(TREE_FILE))?;
    let tags = read_or_empty(&store_dir.join(TAGS_FILE))?;
    let (entries, _) = parse_log(&log);
    let (tree, _) = parse_tree(&tree);
    let config = DecayConfig {
        max_anchors: args.max_anchors,
    };
    let decay = build_decay(&entries, &tree, &decode_rows(&tags), &config);
    println!("{}", render_decay(&decay));
    Ok(())
}
