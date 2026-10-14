use std::collections::{BTreeMap, BTreeSet};

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
    close_line: usize,
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
            close_line: close,
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

fn exit_line(exit: Option<i32>) -> Option<String> {
    match exit {
        Some(0) => None,
        Some(code) => Some(format!("exit {code}")),
        None => Some("killed by signal".to_string()),
    }
}

pub fn display_results(message: &str, runs: &[CellRun]) -> String {
    let closings: Vec<usize> = fences(message)
        .iter()
        .filter(|fence| fence.runtime.is_some())
        .map(|fence| fence.close_line)
        .collect();
    let shown = display_message(message);
    let mut out: Vec<String> = Vec::new();
    for (line, text) in shown.lines().enumerate() {
        out.push(text.to_string());
        if let Some(run) = closings
            .iter()
            .position(|&close| close == line)
            .and_then(|at| runs.get(at))
        {
            if !run.output.is_empty() {
                out.push(fenced("result", &run.output));
            }
            out.extend(exit_line(run.exit));
        }
    }
    let trailing = if shown.ends_with('\n') { "\n" } else { "" };
    format!("{}{trailing}", out.join("\n"))
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    Zsh,
    Bash,
}

impl ShellKind {
    pub fn detect(shell: &str) -> Option<Self> {
        match std::path::Path::new(shell).file_name()?.to_str()? {
            "zsh" => Some(Self::Zsh),
            "bash" => Some(Self::Bash),
            _ => None,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Snapshot {
    env: BTreeMap<String, String>,
    cwd: Option<String>,
    functions: BTreeMap<String, String>,
    aliases: BTreeMap<String, String>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SavedState {
    env: BTreeMap<String, String>,
    unset_env: BTreeSet<String>,
    functions: BTreeMap<String, String>,
    unset_functions: BTreeSet<String>,
    aliases: BTreeMap<String, String>,
    unset_aliases: BTreeSet<String>,
    cwd: Option<String>,
}

fn valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first == '_' || first.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

pub fn parse_snapshots(bytes: &[u8]) -> Option<(Snapshot, Snapshot)> {
    let text = String::from_utf8_lossy(bytes);
    let mut records: Vec<&str> = text.split('\0').collect();
    records.pop();
    let mut done = Vec::new();
    let mut current = Snapshot::default();
    let mut function_name: Option<&str> = None;
    for record in records {
        let mut chars = record.chars();
        let Some(tag) = chars.next() else {
            continue;
        };
        let payload = chars.as_str();
        match tag {
            'E' => {
                if let Some((key, value)) = payload.split_once('=') {
                    if valid_env_name(key) {
                        current.env.insert(key.to_string(), value.to_string());
                    }
                }
            }
            'P' => current.cwd = Some(payload.to_string()),
            'F' => function_name = Some(payload),
            'B' => {
                if let Some(name) = function_name.take() {
                    current
                        .functions
                        .insert(name.to_string(), payload.to_string());
                }
            }
            'A' => {
                if let Some((name, value)) = payload.split_once('=') {
                    current.aliases.insert(name.to_string(), value.to_string());
                }
            }
            'Z' => done.push(std::mem::take(&mut current)),
            _ => {}
        }
    }
    let last = done.pop()?;
    (!done.is_empty()).then(|| (done.swap_remove(0), last))
}

fn changed(
    end: &BTreeMap<String, String>,
    start: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    end.iter()
        .filter(|(key, value)| start.get(*key) != Some(*value))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn removed(end: &BTreeMap<String, String>, start: &BTreeMap<String, String>) -> BTreeSet<String> {
    start
        .keys()
        .filter(|key| !end.contains_key(*key))
        .cloned()
        .collect()
}

pub fn diff_snapshots(end: &Snapshot, start: &Snapshot) -> SavedState {
    SavedState {
        env: changed(&end.env, &start.env),
        unset_env: removed(&end.env, &start.env),
        functions: changed(&end.functions, &start.functions),
        unset_functions: removed(&end.functions, &start.functions),
        aliases: changed(&end.aliases, &start.aliases),
        unset_aliases: removed(&end.aliases, &start.aliases),
        cwd: end.cwd.clone(),
    }
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn restore_script(kind: ShellKind, state: &SavedState) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.extend(
        state
            .unset_env
            .iter()
            .map(|key| format!("unset -v {}", quote(key))),
    );
    lines.extend(
        state
            .env
            .iter()
            .map(|(key, value)| format!("export {key}={}", quote(value))),
    );
    lines.extend(
        state
            .unset_functions
            .iter()
            .map(|name| format!("unset -f -- {}", quote(name))),
    );
    lines.extend(state.functions.iter().map(|(name, body)| match kind {
        ShellKind::Zsh => format!("functions+=({} {})", quote(name), quote(body)),
        ShellKind::Bash => format!("eval {}", quote(body)),
    }));
    lines.extend(
        state
            .unset_aliases
            .iter()
            .map(|name| format!("unalias {} 2>/dev/null", quote(name))),
    );
    lines.extend(
        state
            .aliases
            .iter()
            .map(|(name, value)| format!("alias {}={}", quote(name), quote(value))),
    );
    lines.extend(
        state
            .cwd
            .iter()
            .map(|dir| format!("cd -- {} 2>/dev/null", quote(dir))),
    );
    lines.join("\n")
}

const ZSH_HEAD: &str = r#" unset HISTFILE
precmd_functions=() preexec_functions=() PS1= PS2= PS3= PS4= PROMPT= PROMPT2= RPS1= RPS2= RPROMPT= RPROMPT2= PROMPT_COMMAND=
typeset -g __ralph_v=$options[verbose] __ralph_x=$options[xtrace]
unsetopt verbose xtrace ignoreeof prompt_sp prompt_cr
SAVEHIST=0
unfunction precmd preexec 2>/dev/null
__ralph_dump() {
{ emulate -L zsh; unsetopt xtrace verbose; } 2>/dev/null
local k
{
{
for k in ${(k)parameters[(R)*export*]}; do
[[ $k == (_|PWD|OLDPWD|SHLVL) ]] && continue
print -rN -- "E$k=${(P)k}"
done
print -rN -- "P$PWD"
for k in ${(k)functions}; do
[[ $k == __ralph_* ]] && continue
print -rN -- "F$k" "B${functions[$k]}"
done
for k in ${(k)aliases}; do
print -rN -- "A$k=${aliases[$k]}"
done
print -rN -- Z
} >&3
} 2>/dev/null
}
__ralph_dump"#;

const ZSH_TAIL: &str = r#"zshexit_functions+=(__ralph_dump)
print -rn -- $'\036'
if [[ $__ralph_v == on ]]; then setopt verbose; fi
if [[ $__ralph_x == on ]]; then setopt xtrace; fi
:"#;

const BASH_HEAD: &str = r#"unset HISTFILE
PS1= PS2= PROMPT_COMMAND=
__ralph_flags=$-
set +xv
set +o ignoreeof
set +o history
PROMPT_COMMAND=
__ralph_dump() {
{ set +xv; } 2>/dev/null
local k v
{
{
for k in $(compgen -e); do
case $k in _|PWD|OLDPWD|SHLVL) continue;; esac
printf 'E%s=%s\0' "$k" "${!k}"
done
printf 'P%s\0' "$PWD"
for k in $(compgen -A function); do
case $k in __ralph_*) continue;; esac
printf 'F%s\0B%s\0' "$k" "$(declare -f "$k")"
done
for k in $(compgen -a); do
v=$(alias "$k")
v=${v#alias $k=}
eval "v=$v"
printf 'A%s=%s\0' "$k" "$v"
done
printf 'Z\0'
} >&3
} 2>/dev/null
}
__ralph_dump"#;

const BASH_TAIL: &str = r#"trap __ralph_dump EXIT
case $__ralph_flags in *x*) set -x;; esac
case $__ralph_flags in *v*) set -v;; esac
printf '\036'"#;

const OUTPUT_START: char = '\u{1e}';

pub fn strip_shell_preamble(output: &str) -> &str {
    output
        .split_once(OUTPUT_START)
        .map_or(output, |(_, rest)| rest)
}

fn prologue(kind: ShellKind, state: &SavedState) -> String {
    let (head, tail) = match kind {
        ShellKind::Zsh => (ZSH_HEAD, ZSH_TAIL),
        ShellKind::Bash => (BASH_HEAD, BASH_TAIL),
    };
    let restore = restore_script(kind, state);
    if restore.is_empty() {
        format!("{head}\n{tail}")
    } else {
        format!("{head}\n{restore}\n{tail}")
    }
}

pub fn shell_stdin(kind: ShellKind, state: &SavedState, code: &str) -> String {
    format!("{}\n{code}\n", prologue(kind, state))
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

    fn records(text: &str) -> Vec<u8> {
        text.split('|')
            .flat_map(|part| [part.as_bytes(), b"\0"].concat())
            .collect()
    }

    fn snapshot(env: &[(&str, &str)], pwd: &str) -> Snapshot {
        Snapshot {
            env: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            cwd: Some(pwd.to_string()),
            ..Snapshot::default()
        }
    }

    fn run(code: &str, exit: i32, output: &str) -> CellRun {
        CellRun {
            cell: Cell {
                runtime: "zsh".into(),
                code: code.into(),
            },
            exit: Some(exit),
            output: output.into(),
        }
    }

    #[test]
    fn snapshots_parse_into_start_and_end() {
        let bytes = records("EA=1|EB=x=y|P/a|Ff|Becho f|Azz=ls -l|Z|EA=2|P/b|Z");
        let (start, end) = parse_snapshots(&bytes).unwrap();
        assert_eq!(start.env["B"], "x=y");
        assert_eq!(start.cwd.as_deref(), Some("/a"));
        assert_eq!(start.functions["f"], "echo f");
        assert_eq!(start.aliases["zz"], "ls -l");
        assert_eq!(end.env["A"], "2");
        assert_eq!(end.cwd.as_deref(), Some("/b"));
    }

    #[test]
    fn incomplete_or_single_snapshot_is_unreadable() {
        assert!(parse_snapshots(b"").is_none());
        assert!(parse_snapshots(&records("EA=1|Z")).is_none());
        assert!(parse_snapshots(&records("EA=1|Z|EA=2")).is_none());
    }

    #[test]
    fn bad_env_names_are_dropped() {
        let (start, _) = parse_snapshots(&records("EA B=1|E=2|E1X=3|EOK=4|Z|Z")).unwrap();
        assert_eq!(start.env.keys().collect::<Vec<_>>(), vec!["OK"]);
    }

    #[test]
    fn diff_keeps_new_changed_and_removed_and_drops_inherited() {
        let start = snapshot(
            &[("SECRET", "s"), ("KEEP", "1"), ("GONE", "g"), ("EDIT", "a")],
            "/rc",
        );
        let mut end = snapshot(
            &[("SECRET", "s"), ("KEEP", "1"), ("EDIT", "b"), ("NEW", "n")],
            "/tmp",
        );
        end.aliases.insert("zz".into(), "ls".into());
        let saved = diff_snapshots(&end, &start);
        assert_eq!(
            saved.env.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["EDIT", "NEW"]
        );
        assert_eq!(saved.unset_env.iter().collect::<Vec<_>>(), vec!["GONE"]);
        assert_eq!(saved.aliases["zz"], "ls");
        assert_eq!(saved.cwd.as_deref(), Some("/tmp"));
    }

    #[test]
    fn restore_quotes_quotes_newlines_and_unicode() {
        let state = SavedState {
            env: [("K".to_string(), "it's\nñ \"$x\"".to_string())].into(),
            unset_env: ["OLD".to_string()].into(),
            aliases: [("a".to_string(), "echo 'hi'".to_string())].into(),
            cwd: Some("/tmp/it's".to_string()),
            ..SavedState::default()
        };
        assert_eq!(
            restore_script(ShellKind::Zsh, &state),
            "unset -v 'OLD'\nexport K='it'\\''s\nñ \"$x\"'\nalias 'a'='echo '\\''hi'\\'''\ncd -- '/tmp/it'\\''s' 2>/dev/null"
        );
    }

    #[test]
    fn functions_restore_per_shell() {
        let state = SavedState {
            functions: [("f".to_string(), "echo f".to_string())].into(),
            ..SavedState::default()
        };
        assert_eq!(
            restore_script(ShellKind::Zsh, &state),
            "functions+=('f' 'echo f')"
        );
        assert_eq!(restore_script(ShellKind::Bash, &state), "eval 'echo f'");
    }

    #[test]
    fn empty_state_prologue_carries_nothing() {
        for kind in [ShellKind::Zsh, ShellKind::Bash] {
            let prologue = prologue(kind, &SavedState::default());
            for marker in [
                "cd --", "unset -v", "unset -f", "unalias", "alias '", "export ",
            ] {
                assert!(!prologue.contains(marker), "{kind:?} has {marker}");
            }
            assert!(prologue.contains("unset HISTFILE"));
        }
    }

    #[test]
    fn preamble_noise_before_the_marker_is_dropped() {
        assert_eq!(strip_shell_preamble("rc noise\n\u{1e}hi\n"), "hi\n");
        assert_eq!(strip_shell_preamble("hi\n"), "hi\n");
    }

    #[test]
    fn shell_stdin_ends_with_the_code() {
        let stdin = shell_stdin(ShellKind::Bash, &SavedState::default(), "echo hi");
        assert!(stdin.ends_with("\nprintf '\\036'\necho hi\n"));
    }

    #[test]
    fn shell_kind_comes_from_the_file_name() {
        assert_eq!(ShellKind::detect("/bin/zsh"), Some(ShellKind::Zsh));
        assert_eq!(
            ShellKind::detect("/usr/local/bin/bash"),
            Some(ShellKind::Bash)
        );
        assert_eq!(ShellKind::detect("/bin/sh"), None);
        assert_eq!(ShellKind::detect("/usr/bin/fish"), None);
    }

    #[test]
    fn results_follow_each_cell_untrimmed_with_exit_line() {
        let runs = [
            run("a", 0, "1\n2\n"),
            run("b", 3, "```x\n"),
            run("c", 1, ""),
        ];
        let message = "go\n```!zsh\na\n```\nmid\n```!zsh\nb\n```\n```!zsh\nc\n```\n";
        assert_eq!(
            display_results(message, &runs),
            "go\n```zsh\na\n```\n```result\n1\n2\n```\nmid\n```zsh\nb\n```\n````result\n```x\n````\nexit 3\n```zsh\nc\n```\nexit 1\n"
        );
    }
}
