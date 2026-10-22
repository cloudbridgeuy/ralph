# Mutation testing

ralph uses [cargo-mutants](https://mutants.rs/) to check that the unit tests of pure logic actually assert behaviour. Phase 1 covers pure `ralph_core` modules only. Full runs are local and manual; no CI job runs mutation tests (see [PR CI](#pr-ci-planned-disabled)).

## Setup

```sh
cargo install cargo-mutants --version 27.1.0 --locked
```

The version is pinned. cargo-mutants 27.1.0 requires Rust 1.88, below the workspace toolchain 1.95.0. Output goes to `mutants.out/` and `mutants.out.old/`, both ignored by git.

## Commands

Plain `cargo mutants` is the entry point. There is no xtask wrapper, and lint and hooks do not run it.

```sh
# Full phase-1 run (local/manual only)
cargo mutants -p ralph_core -j 2 --build-timeout 300
```

`-j 2` means two mutation workers, each with its own scratch copy, not two test threads.

Extras:

```sh
cargo mutants --list-files                  # files in scope
cargo mutants --list                        # every mutant, no build
cargo mutants --in-diff changes.diff -p ralph_core -j 2 --build-timeout 300
cargo mutants --shard 1/10 -p ralph_core -j 2 --build-timeout 300
```

`--in-diff` mutates only lines touched by a diff, intersected with the configured scope. `--shard k/n` runs a deterministic slice, which is useful to sample timings or split a full run across sessions.

## Scope

`.cargo/mutants.toml` is an allow-list. New files stay out until someone adds them, so shell code is never mutated by accident.

Examined:

- `crates/core/src/shell_format.rs`
- `crates/core/src/status_segments.rs`
- `crates/core/src/chat_status.rs`
- `crates/core/src/chat_cell.rs`
- `crates/core/src/worker_check.rs`
- `crates/core/src/chat_memory/tree.rs`
- `crates/core/src/stream/**/*.rs`
- `crates/core/src/chunk/**/*.rs`
- `crates/core/src/wire/**/*.rs`
- `crates/core/src/worker.rs`, `worker/worktree.rs`, `worker/rollback.rs`
- `crates/core/src/watch.rs`, `crates/core/src/watch_store.rs` (time is passed in, not read)

Excluded: `.worktrees/**`, `.claude/worktrees/**`, `vendor/**`, `target/**`, `xtask/**`, `**/tests/**`, `**/tests.rs`. Test files are not mutated but still run. `exclude_re` holds five line-pinned equivalents, listed in the [Ledger](#ledger).

`wire/**` is not in the design's list. It was added in GUZ-617 because a grep found no filesystem, process, environment or clock calls in it.

Open: the `worker.rs` children `kind`, `markers`, `ownership` and `files` have not been judged pure or impure. The ACP and HTTP children stay out on purpose.

`additional_cargo_test_args = ["--all-targets"]` runs library and binary unit tests and skips doctests; normal test CI still runs doctests. `ralph_core` does not depend on the patched `vendor/reedline`, and the exclusions do not remove it from scratch builds.

## Timeouts and baseline

Local runs always run the baseline (unmutated build and tests) first. The config sets no fixed seconds:

- Test limit: `max(30 s, 5 × baseline test time)` (`minimum_test_timeout = 30.0`, `timeout_multiplier = 5.0`).
- Build limit: 300 s, passed as `--build-timeout 300`.

A hung baseline test is not covered by these limits; interrupt it by hand. Only the planned CI job would skip the baseline, after the same SHA passes the separate test job. Never combine `--baseline skip` with the multiplier, since the multiplier then has no baseline to scale.

## PR CI (planned, disabled)

CI mutation testing is on hold under GUZ-620. The design requires all runnable job work — checkout, diff, tool and cache provisioning, scratch build, enumeration, tests, cleanup and reporting — to fit a hard **45-second** cap. Useful work stops at 38 s, process groups are killed by 40 s, and the job finishes by 45 s. GitHub's `timeout-minutes: 1` is a deadman only.

If the job ever runs, it would:

- start only after the separate test job passed on the exact checked-out SHA, then use `--baseline skip`;
- mutate only the PR's three-dot diff within the allow-list, with a pinned prebuilt cargo-mutants;
- report "no in-scope mutants" for an empty selection;
- on deadline exhaustion report **INCOMPLETE: 45s budget** with attempted/remaining counts and captured outcomes, never a green "all caught";
- stay advisory: no required check, no score gate, and tool or config failures reported apart from survivors.

The design's per-mutant limits (`--timeout 3 --build-timeout 5`) are below measured values (test median 3.39 s, first build 10–12 s) and are too low as written. The GUZ-617 measurements show that useful feedback does not fit 45 s. Whether to proceed is the person's decision; the budget is never relaxed to make it fit.

## Results

Evidence: [GUZ-617 measurements](https://linear.app/guzmonne/issue/GUZ-617/measure-mutation-cost-and-add-cargo-mutants-configuration#comment-db49f189) and the [GUZ-620 hold](https://linear.app/guzmonne/issue/GUZ-620/add-advisory-pr-diff-mutation-ci-with-a-hard-45-second-cap#comment-92ea2e13).

Measured, Apple M2 Pro (12 cores, 32 GiB), Rust 1.95.0, cargo-mutants 27.1.0, warm `~/.cargo` registry; "cold" means an empty target dir:

| Item | Value | Notes |
| --- | --- | --- |
| Scope at d3bfae5 | 41 files, 1,642 mutants | `--list-files`, `--list` |
| Enumeration | 0.35–0.39 s | SHA 3f2a4e5 |
| Per mutant (n=110, `--shard 1/10`) | median 5.69 s, p95 6.85 s, max 32.2 s | build 2.30/3.09 s, test 3.39/3.65 s |
| Sample outcomes | 92 caught, 7 missed, 9 unviable, 2 timeouts | 6 min wall |
| 6-mutant diff, cold, with baseline, `-j 2` | 42.5 s | baseline 11.6 s build + 3.5 s test |
| 6-mutant diff, warm, `--copy-target --baseline skip`, `-j 2` | 27.1 s | first build per worker 11 s |
| 6-mutant diff, warm, `--in-place --baseline skip`, `-j 1` | 32.1 s | |
| Cold `cargo test -p ralph_core --no-run` | 10.1 s | target 243 MB |

The per-mutant and diff rows were measured at SHA 3f2a4e5, before `wire/**` joined the scope (24 files, 1,100 mutants then).

Estimates, not measured:

| Item | Value |
| --- | --- |
| CI setup + first build (ubuntu-24.04, 4 vCPU) | 30–45 s before the first mutant |
| CI cost per mutant per worker | 8–12 s |

First full local core run (GUZ-618):

| Item | Value |
| --- | --- |
| SHA | 0c0085b |
| Command | `cargo mutants -p ralph_core -j 2 --build-timeout 300`, with baseline |
| Machine | Apple M2 Pro, 12 cores, 32 GiB; Rust 1.95.0; cargo-mutants 27.1.0 |
| Baseline build / test | 10.95 s / 3.29 s (test limit is the 30 s minimum) |
| Wall time | 83 min (4,951 s) |
| Per mutant (n=1,642) | median 5.59 s, p95 6.53 s, max 36.1 s |
| Caught / missed / unviable / timeouts | 1,357 / 90 / 152 / 43 |

After GUZ-618:

| Item | Value |
| --- | --- |
| Killed by new unit tests | 58 missed mutants |
| Targeted rerun, wire lines | 68 mutants: 67 caught, 1 equivalent |
| Targeted rerun, other lines | 70 mutants: 67 caught, 1 unviable, 2 timeouts |
| Deleted condition | `shell_format.rs:234` `\|\| open_group` was redundant |
| Rerun `-F 'shell_format\.rs:234:'` | 18 mutants: 18 caught |
| Excluded equivalents | 5 |
| Deferred | 26 `shell_format.rs` survivors, [GUZ-811](https://linear.app/guzmonne/issue/GUZ-811) |
| Targeted rerun, cache tier `Display` and call sites | 32 mutants: 32 caught |
| Scope after | 1,623 mutants |

## Survivors

After a run, review `mutants.out/missed.txt` with the diffs and logs. Classify each survivor as a missing assertion, an equivalent mutant, dead code, or a scope/infrastructure error:

- Missing assertion: add a pure unit test.
- Dead code: delete it.
- Equivalent: add an exact `exclude_re` pattern, confirm with `cargo mutants --list` that it removes only that mutant, and add a ledger entry.
- Unresolved: link a follow-up issue.

Do not use `#[mutants::skip]`, broad trait or default skips, or a score threshold. Timeouts and unviable mutants are tracked apart from survivors; they say nothing about test strength.

### Ledger

| Mutant | Reason | Owner | Review trigger |
| --- | --- | --- | --- |
| `sse.rs:23:34` `<` to `<=` in `SseDecoder::push` | A 3-byte BOM-only buffer waits for the next push. No other method reads the buffer, so events are identical. | GUZ-618 | Line moves, or the BOM check or a flush/finish method changes. |
| `sse.rs:23:46` `&&` to `\|\|` in `SseDecoder::push` | Same reason. A short non-BOM buffer yields no events either way. | GUZ-618 | Same as above. |
| `tree.rs:14:28` `>` to `>=` in `cover_at` | When `hi == t`, size > 1 >= alpha, so both forms split. | GUZ-618 | Line moves, or `cover_at` accepts alpha > 1. |
| `shell_format.rs:356:25` `>` to `>=` in `Layout::statement` | A one-stage loop makes the same `push_stage` call as the single-stage branch. | GUZ-618 | Line moves, or the two branches diverge. |
| `anthropic.rs` `secret_headers` to `Vec::leak(Vec::new())` | The original returns `&[]`. | GUZ-618 | Anthropic gets provider-specific secret headers. |

The regexes are pinned to file:line:col. Any edit that moves a line drops the exclusion and the mutant reappears as missed. Re-pin it after checking `cargo mutants --list`.

Deferred: 26 `shell_format.rs` missing assertions in `scan`, `Layout::push_stage`, `Layout::statement` and `Layout::segment` go to [GUZ-811](https://linear.app/guzmonne/issue/GUZ-811).

### Timeouts and unviable

43 timeouts, all mutations that stop a loop advancing (`+=` to `*=`/`-=`, `next_line`/`next` returning `Some(..)`, and similar). They hit the 30 s limit and are expected.

| File | Timeouts |
| --- | --- |
| `chat_memory/tree.rs` | 15 |
| `shell_format.rs` | 10 |
| `wire/sse.rs` | 8 |
| `chat_cell.rs` | 4 |
| `chunk/streaming.rs` | 3 |
| `wire/diff.rs` | 1 |
| `watch.rs` | 1 |
| `status_segments.rs` | 1 |

9 in `tree.rs` (`block_line`, `node_line`, `split_target`, `mergeable_pair`) are judged infinite loops from the code. The logs do not name the hung test.

152 unviable mutants do not compile. They are counted only.

## Rollout

**Phase 1** is done only after a full local core run with a healthy baseline, real counts and timing in [Results](#results), classified survivors and timeouts, linked follow-ups, and unit tests for the prioritised real gaps. GUZ-618 met these criteria; remaining gaps are in [GUZ-811](https://linear.app/guzmonne/issue/GUZ-811).

**Phase 2 (planned)** adds these `crates/ralph/src` files: `stream_processor/{types,output_block,block_builders,utils}.rs`, `render/{wrap,utils}.rs`, `markdown.rs`, `ansi.rs`, `frame.rs`, `formatting.rs`, `status_theme.rs`. The global-state `cap_wrap_width` and `wrap_width` are excluded with list-verified regexes. `render/text.rs`, `diff_highlight.rs` and the rest of the stream processor stay out because they read the terminal, spawn tools or touch the filesystem. Phase 2 has the same completion standard and runs locally:

```sh
cargo mutants -p ralph -j 2 --build-timeout 300
```
