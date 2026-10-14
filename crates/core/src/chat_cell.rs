pub const CELL_OUTPUT_LINES: usize = 200;
pub const CELL_OUTPUT_BYTES: usize = 65536;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub runtime: String,
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellRun {
    pub cell: Cell,
    pub exit: Option<i32>,
    pub output: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellLimits {
    pub lines: Option<usize>,
    pub bytes: Option<usize>,
}

pub fn resolve_cell_cap(
    name: &str,
    env: Option<&str>,
    config: Option<usize>,
    default: usize,
) -> Result<Option<usize>, String> {
    let env = env
        .map(|value| {
            value
                .trim()
                .parse()
                .map_err(|_| format!("{name} {value:?} is not a whole number."))
        })
        .transpose()?;
    Ok(Some(env.or(config).unwrap_or(default)).filter(|&cap| cap > 0))
}

pub fn resolve_cell_limits(
    lines_env: Option<&str>,
    bytes_env: Option<&str>,
    lines_config: Option<usize>,
    bytes_config: Option<usize>,
) -> Result<CellLimits, String> {
    Ok(CellLimits {
        lines: resolve_cell_cap(
            "cell output limit",
            lines_env,
            lines_config,
            CELL_OUTPUT_LINES,
        )?,
        bytes: resolve_cell_cap(
            "cell output bytes",
            bytes_env,
            bytes_config,
            CELL_OUTPUT_BYTES,
        )?,
    })
}

struct Fence {
    open_line: usize,
    runtime: Option<String>,
    code: String,
}

fn backtick_run(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    let ticks = trimmed.len() - trimmed.trim_start_matches('`').len();
    (ticks >= 3).then(|| (ticks, trimmed[ticks..].trim()))
}

fn cell_runtime(info: &str) -> Option<String> {
    let runtime = info.strip_prefix('!')?;
    (!runtime.is_empty() && !runtime.contains(char::is_whitespace)).then(|| runtime.to_string())
}

fn fences(message: &str) -> Vec<Fence> {
    let lines: Vec<&str> = message.lines().collect();
    let mut found = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let Some((ticks, info)) = backtick_run(lines[index]) else {
            index += 1;
            continue;
        };
        let close = (index + 1..lines.len()).find(|&at| {
            backtick_run(lines[at]).is_some_and(|(run, rest)| run >= ticks && rest.is_empty())
        });
        let Some(close) = close else {
            break;
        };
        found.push(Fence {
            open_line: index,
            runtime: cell_runtime(info),
            code: lines[index + 1..close].join("\n"),
        });
        index = close + 1;
    }
    found
}

pub fn parse_cells(message: &str) -> Vec<Cell> {
    fences(message)
        .into_iter()
        .filter_map(|fence| {
            fence.runtime.map(|runtime| Cell {
                runtime,
                code: fence.code,
            })
        })
        .collect()
}

pub fn display_message(message: &str) -> String {
    let opening: Vec<usize> = fences(message)
        .iter()
        .filter(|fence| fence.runtime.is_some())
        .map(|fence| fence.open_line)
        .collect();
    let mut shown: Vec<String> = message.lines().map(str::to_string).collect();
    for line in opening {
        shown[line] = shown[line].replacen('!', "", 1);
    }
    let trailing = if message.ends_with('\n') { "\n" } else { "" };
    format!("{}{trailing}", shown.join("\n"))
}

fn floor_boundary(text: &str, mut at: usize) -> usize {
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

fn ceil_boundary(text: &str, mut at: usize) -> usize {
    while !text.is_char_boundary(at) {
        at += 1;
    }
    at
}

fn trim_lines(output: &str, limit: usize) -> String {
    let lines: Vec<&str> = output.split_inclusive('\n').collect();
    if lines.len() <= limit {
        return output.to_string();
    }
    let head = limit / 2;
    let tail = limit - head;
    format!(
        "{}… {} lines trimmed …\n{}",
        lines[..head].concat(),
        lines.len() - limit,
        lines[lines.len() - tail..].concat()
    )
}

fn trim_bytes(output: &str, limit: usize) -> String {
    if output.len() <= limit {
        return output.to_string();
    }
    let head_end = floor_boundary(output, limit / 2);
    let tail_start = ceil_boundary(output, output.len() - (limit - limit / 2));
    let tail_start = tail_start.max(head_end);
    format!(
        "{}\n… {} bytes trimmed …\n{}",
        &output[..head_end],
        tail_start - head_end,
        &output[tail_start..]
    )
}

pub fn trim_output(output: &str, limits: CellLimits) -> String {
    let by_lines = match limits.lines {
        Some(limit) => trim_lines(output, limit),
        None => output.to_string(),
    };
    match limits.bytes {
        Some(limit) => trim_bytes(&by_lines, limit),
        None => by_lines,
    }
}

fn fence_for(text: &str) -> String {
    let longest = text
        .lines()
        .filter_map(backtick_run)
        .map(|(ticks, _)| ticks)
        .max()
        .unwrap_or(0);
    "`".repeat((longest + 1).max(3))
}

fn fenced(info: &str, body: &str) -> String {
    let fence = fence_for(body);
    format!("{fence}{info}\n{}\n{fence}", body.trim_end_matches('\n'))
}

pub fn build_message(message: &str, runs: &[CellRun], limits: CellLimits) -> String {
    let results: Vec<String> = runs
        .iter()
        .enumerate()
        .map(|(index, run)| {
            let exit = run
                .exit
                .map_or_else(|| "killed by signal".to_string(), |code| code.to_string());
            format!(
                "[cell {} · {} · exit {exit}]\n{}\noutput:\n{}",
                index + 1,
                run.cell.runtime,
                fenced(&run.cell.runtime, &run.cell.code),
                fenced("", &trim_output(&run.output, limits))
            )
        })
        .collect();
    format!(
        "{}\n\n{}",
        display_message(message).trim_end(),
        results.join("\n\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: CellLimits = CellLimits {
        lines: None,
        bytes: None,
    };

    fn numbered(count: usize) -> String {
        (1..=count).map(|n| format!("{n}\n")).collect()
    }

    #[test]
    fn parses_cells_in_order_and_skips_plain_fences() {
        let message = "run\n```!zsh\nls\n```\n```rust\nfn main() {}\n```\n````!python3\nprint(1)\n```\n````\n";
        let cells = parse_cells(message);
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].runtime, "zsh");
        assert_eq!(cells[0].code, "ls");
        assert_eq!(cells[1].runtime, "python3");
        assert_eq!(cells[1].code, "print(1)\n```");
    }

    #[test]
    fn cell_inside_another_fence_is_not_a_cell() {
        assert!(parse_cells("````md\n```!zsh\nls\n```\n````").is_empty());
        assert!(parse_cells("```!zsh\nls").is_empty());
        assert!(parse_cells("```!\nls\n```").is_empty());
        assert!(parse_cells("```!a b\nls\n```").is_empty());
    }

    #[test]
    fn display_drops_the_bang_only_on_cell_fences() {
        let message = "a ! b\n```!zsh\necho !x\n```\n";
        assert_eq!(display_message(message), "a ! b\n```zsh\necho !x\n```\n");
        assert_eq!(display_message("plain"), "plain");
    }

    #[test]
    fn under_and_exactly_at_the_line_limit_pass_through() {
        let limits = CellLimits {
            lines: Some(4),
            bytes: None,
        };
        assert_eq!(trim_output(&numbered(3), limits), numbered(3));
        assert_eq!(trim_output(&numbered(4), limits), numbered(4));
    }

    #[test]
    fn over_the_line_limit_keeps_half_head_half_tail() {
        let limits = CellLimits {
            lines: Some(4),
            bytes: None,
        };
        assert_eq!(
            trim_output(&numbered(10), limits),
            "1\n2\n… 6 lines trimmed …\n9\n10\n"
        );
    }

    #[test]
    fn one_huge_line_is_cut_by_bytes() {
        let limits = CellLimits {
            lines: Some(200),
            bytes: Some(10),
        };
        let trimmed = trim_output(&"a".repeat(100), limits);
        assert_eq!(trimmed, "aaaaa\n… 90 bytes trimmed …\naaaaa");
    }

    #[test]
    fn bytes_cut_never_splits_a_char() {
        let limits = CellLimits {
            lines: None,
            bytes: Some(9),
        };
        let trimmed = trim_output(&"é".repeat(20), limits);
        assert!(trimmed.contains("bytes trimmed"));
        assert!(trimmed.ends_with('é'));
    }

    #[test]
    fn both_caps_apply_and_zero_disables() {
        let limits = CellLimits {
            lines: Some(4),
            bytes: Some(20),
        };
        let trimmed = trim_output(&numbered(1000), limits);
        assert!(trimmed.contains("bytes trimmed"));
        assert!(trimmed.ends_with("1000\n"));
        let off = resolve_cell_limits(None, None, Some(0), Some(0)).unwrap();
        assert_eq!(off, NONE);
        assert_eq!(trim_output(&numbered(1000), off), numbered(1000));
    }

    #[test]
    fn cap_resolution_prefers_env_then_config_then_default() {
        let defaults = resolve_cell_limits(None, None, None, None).unwrap();
        assert_eq!(defaults.lines, Some(200));
        assert_eq!(defaults.bytes, Some(65536));
        let limits = resolve_cell_limits(Some(" 7 "), None, Some(9), Some(11)).unwrap();
        assert_eq!(limits.lines, Some(7));
        assert_eq!(limits.bytes, Some(11));
        assert!(resolve_cell_limits(Some("many"), None, None, None).is_err());
    }

    #[test]
    fn message_carries_code_exit_and_output() {
        let run = CellRun {
            cell: Cell {
                runtime: "zsh".into(),
                code: "echo hi".into(),
            },
            exit: Some(3),
            output: "hi\n".into(),
        };
        assert_eq!(
            build_message("go\n```!zsh\necho hi\n```", &[run], NONE),
            "go\n```zsh\necho hi\n```\n\n[cell 1 · zsh · exit 3]\n```zsh\necho hi\n```\noutput:\n```\nhi\n```"
        );
    }
}
