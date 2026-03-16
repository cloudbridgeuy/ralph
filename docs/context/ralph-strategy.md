# ralph strategy Command

This document describes the `ralph strategy` command, which manages and executes collaboration strategies — predefined patterns of persona interaction defined in TOML files.

## Code Locations

| Component | File | Description |
|-----------|------|-------------|
| CLI args | `crates/ralph/src/cli/mod.rs` | StrategyArgs, StrategyAction, StrategyExecuteArgs |
| Core types | `crates/core/src/strategy.rs` | StrategyConfig, TeamStrategy, StrategyError, parsing, validation |
| Discovery & display | `crates/ralph/src/strategy/mod.rs` | File discovery, loading, validation, formatting, `load_team_strategy()` |
| Bundled assets | `crates/ralph/src/strategy/assets.rs` | Compile-time embedded agent files and strategy template via `include_str!` |
| Init command | `crates/ralph/src/strategy/init.rs` | FC-IS init: `plan_file_actions()` (pure), `format_init_summary()` (pure), `execute_init()` (shell) |
| Execution dispatcher | `crates/ralph/src/strategy/execute.rs` | Strategy kind dispatch, run orchestration |
| Strategy trait | `crates/ralph/src/strategy/traits.rs` | Strategy trait, StrategyExecutionContext, run_strategy() |
| PrdLoop strategy | `crates/ralph/src/strategy/prd_loop.rs` | Self-contained PRD iteration loop with orchestration |
| ConversationLoop strategy | `crates/ralph/src/strategy/conversation_loop.rs` | Human-driven conversation loop with agent collaboration |
| Transcript types | `crates/core/src/transcript.rs` | Speaker, TranscriptEntry, HumanResponse, editor content building |
| Human I/O | `crates/ralph/src/human.rs` | Editor interaction, comment display, human-in-the-loop I/O |
| Human classification | `crates/core/src/human.rs` | DirectiveTarget, classify_target, partition_directives |
| Recovery | `crates/ralph/src/recovery.rs` | Subprocess failure recovery, InvocationConfig, retry logic |
| CLI tests | `crates/ralph/src/cli/tests/strategy.rs` | CLI argument parsing tests |
| Main wiring | `crates/ralph/src/main.rs` | execute_strategy(), execute_strategy_init(), execute_strategy_list(), execute_strategy_execute() |
| Bundled agents | `assets/agents/*.md` | Default agent markdown files scaffolded by `ralph strategy init` |
| Bundled strategies | `assets/strategies/*.toml` | Default strategy TOML files scaffolded by `ralph strategy init` |
| Default strategy | `assets/strategy.toml` | Default `.claude/strategy.toml` template |

## Purpose

Strategies define structured collaboration patterns between personas. Each strategy lives in a `.claude/strategies/*.toml` file and maps to a hardcoded Rust implementation via its `kind` field. Strategies specify which persona executes iterations (`primary_persona`), which personas are reachable via orchestration (`available_personas`), and additional instructions (`prompt_aggregates`).

## Strategy TOML Schema

```toml
name = "prd-loop"
description = "Iterates through PRD stories using a developer persona"
kind = "prd-loop"
primary_persona = "developer"
available_personas = ["architect", "reviewer", "tester"]
prompt_aggregates = [
  "Focus on one story at a time.",
  "Run cargo xtask lint before claiming any story is done.",
]
```

### Fields

| Field | Required | Type | Description |
|-------|----------|------|-------------|
| `name` | yes | string | Human-readable strategy name |
| `description` | yes | string | What the strategy does |
| `kind` | yes | string | Maps to a Rust implementation (e.g., `"prd-loop"`) |
| `primary_persona` | yes | string | Persona that executes each iteration |
| `available_personas` | no | string[] | Personas reachable via orchestration directives |
| `prompt_aggregates` | no | string[] | Additional instructions appended to persona prompt |

## Project Strategy File (`.claude/strategy.toml`)

Separate from the per-strategy execution configs in `.claude/strategies/*.toml`, the project-level strategy file `.claude/strategy.toml` defines the team structure and agent definitions for a project. It is created by `ralph strategy init` and consumed by `ralph persona <name>` for agent resolution.

### Schema

```toml
[agents]
architect = ".claude/agents/architect.md"
developer = ".claude/agents/developer.md"
reviewer = ".claude/agents/reviewer.md"
tester = ".claude/agents/tester.md"
product-manager = ".claude/agents/product-manager.md"
```

The `[agents]` table maps agent names to markdown file paths relative to the project root. Each entry is a key-value pair where:

- **Key**: the agent name used with `ralph persona <name>`
- **Value**: path to the agent's markdown file (relative to project root)

### Resolution Order

When `ralph persona <name>` is invoked:

1. If `.claude/strategy.toml` exists, resolve `<name>` against the `[agents]` table
2. Load the agent markdown file from the path specified in the strategy
3. If no strategy file exists, fall back to existing behavior (scan `.claude/agents/` and `~/.claude/agents/`)

## CLI Subcommands

### `ralph strategy init`

Scaffolds a default project strategy with agent definitions and bundled strategy files. Creates `.claude/agents/` and `.claude/strategies/` directories with default files, plus a `.claude/strategy.toml` defining the team structure.

**What it creates:**

| File | Description |
|------|-------------|
| `.claude/agents/architect.md` | Default architect agent definition |
| `.claude/agents/developer.md` | Default developer agent definition |
| `.claude/agents/reviewer.md` | Default reviewer agent definition |
| `.claude/agents/tester.md` | Default tester agent definition |
| `.claude/agents/product-manager.md` | Default product manager agent definition |
| `.claude/strategies/prd-loop.toml` | Default prd-loop strategy definition |
| `.claude/strategy.toml` | Project strategy file referencing all agents |

**Behavior:**

- Creates `.claude/agents/` and `.claude/strategies/` directories if they don't exist
- Only creates missing files — existing agent and strategy files are not overwritten
- Fails with a clear error if `.claude/strategy.toml` already exists (no silent overwrite)
- Prints confirmation summarizing what was created vs skipped

**Bundled assets** are stored in `assets/agents/` and `assets/strategies/` in the ralph source tree and compiled into the binary via `include_str!`. The init command writes file copies to disk — users own these files and can customize them freely.

### `ralph strategy list`

Discovers strategy files from `.claude/strategies/*.toml`, validates them against known kinds and available personas, and displays a columnar table.

### `ralph strategy execute <name>`

Looks up a strategy by name, validates it, and invokes the corresponding Rust implementation.

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `<NAME>` | positional | required | Strategy name to execute |
| `--max-iterations` | usize | auto | Maximum iterations (strategy-dependent) |
| `--resume` | flag | false | Resume a previously stopped session |

## Architecture (FC-IS)

**Functional Core** (`crates/core/src/strategy.rs`):
- `parse_strategy()` — parse TOML content into `StrategyConfig`
- `parse_team_strategy()` — parse `.claude/strategy.toml` content into `TeamStrategy`
- `validate_personas()` — check persona references against discovered agents
- `validate_prompt_aggregates()` — reject empty string entries

**Functional Core** (`crates/ralph/src/strategy/init.rs`):
- `plan_file_actions()` — decide create/skip actions for any asset set based on file existence flags
- `format_action()` — format a single init action as a summary line
- `format_init_summary()` — format init results for display (agents + strategies)

**Imperative Shell** (`crates/ralph/src/strategy/`):
- `mod.rs` — `discover_strategies()`, `load_and_validate_strategy()`, `load_all_strategies()`, `load_team_strategy()`, `find_strategy_by_name()`, `format_strategy_list()`
- `init.rs` — `execute_init()` orchestrates directory creation, file writes, and summary display; `check_existing()`, `write_planned_files()`, `create_dir()` as I/O helpers
- `assets.rs` — compile-time embedded agent files, strategy files, and strategy template
- `execute.rs` — `execute_strategy_execute()` dispatcher, matches on `StrategyKind` enum
- `traits.rs` — `Strategy` trait with `execute()` and `between_iterations()` hooks, `StrategyExecutionContext`, `run_strategy()` generic dispatcher
- `prd_loop.rs` — `PrdLoopStrategy` implements `Strategy` trait with self-contained iteration loop: session management, subprocess invocation via `recovery::invoke_with_failure_recovery`, completion detection, orchestration directive scanning, and keyboard control propagation

### PrdLoop Internal Types

| Type | Purpose |
|------|---------|
| `StrategyInvocationConfig` | Groups read-only loop parameters (subsumes old `run::InvocationConfig` pattern) |
| `StrategyIdentity` | Groups strategy name + persona for log entries |
| `AccumulatedMetrics` | Tracks cost, duration, and token usage across iterations |
| `IterationState` | Mutable state during the iteration loop |

### ConversationLoop Internal Types

| Type | Purpose |
|------|---------|
| `ConversationLoopConfig` | Groups read-only loop parameters (persona, timeouts, available personas) |
| `AccumulatedMetrics` | Tracks cost, duration, and token usage across iterations |
| `LoopState` | Mutable state: transcript, iterations, metrics, key action |

### ConversationLoop Flow

1. Initialize session
2. **Outer loop**: open `$EDITOR` with transcript + separator
3. Parse human response below separator
4. If empty/abort → exit
5. Build persona prompt with conversation history
6. Invoke persona via `invoke_with_failure_recovery`
7. Extract response text, handle directives (comments, human asks, persona orchestration)
8. Append both entries to transcript
9. Repeat from step 2

## Strategy Kind Dispatch

Strategies use enum dispatch via `StrategyKind` (defined in core). The `kind` string from TOML is resolved to a typed enum variant at load time using `resolve_kind()` (Parse Don't Validate pattern). The dispatcher in `execute.rs` uses exhaustive `match` so missing implementations are caught at compile time.

```
main.rs::execute_strategy_execute(args)
  -> strategy::execute::execute_strategy_execute(args)
       -> load_all_strategies()
       -> find_strategy_by_name()
       -> build_execution_context() -> StrategyExecutionContext
       -> match StrategyKind {
            PrdLoop -> run_strategy(&PrdLoopStrategy, ctx)
                         -> execute_prd_loop() [session lifecycle, iteration loop, orchestration]
            ConversationLoop -> run_strategy(&ConversationLoopStrategy, ctx)
                                 -> execute_conversation_loop() [editor loop, human I/O, orchestration]
          }
```

## Known Kinds

Currently registered: `prd-loop`, `conversation-loop`

### `prd-loop`

Automated iteration over PRD stories. The primary persona executes iterations, checks completion against the PRD, and stops when all stories are done or max iterations are reached.

### `conversation-loop`

Human-driven conversation loop. The human writes in `$EDITOR`, the primary persona responds, and the loop repeats. Supports:

- **Human-in-the-loop directives**: Personas can ask the human directly via `<ralph-ask to="human">` (opens editor) or send comments via `<ralph-comment to="human">` (terminal soft-block).
- **Agent-to-agent orchestration**: The primary persona can delegate to available personas using standard `<ralph-ask>` and `<ralph-handover>` directives.
- **Transcript accumulation**: The full conversation history is shown in the editor and sent to the persona with each invocation.

**Exit conditions**: Empty editor response, `--max-iterations` limit, keyboard interrupt.

**Example strategy** (`fiction-loop`):
```toml
name = "fiction-loop"
description = "Collaborative fiction writing with human-in-the-loop"
kind = "conversation-loop"
primary_persona = "storyteller"
available_personas = ["editor-agent", "worldbuilder", "critic"]
```

## Human-in-the-Loop Directives

Personas can interact with the human operator using the reserved target `"human"`:

| Directive | Behavior |
|-----------|----------|
| `<ralph-ask to="human">question</ralph-ask>` | Opens `$EDITOR` with the question as context. Human's response is fed back to the persona. |
| `<ralph-comment to="human">message</ralph-comment>` | Displays the message in the terminal and soft-blocks until the human presses Enter or types a response. |

These directives work in any strategy kind, but are most useful in `conversation-loop` where the human is actively participating.

The `Comment` verb is separated from `Ask`/`Handover` directives before validation via `extract_comments()`, so comments can coexist with other directive types in a single response.

New strategies require:
1. Adding a variant to `StrategyKind` in `crates/core/src/strategy.rs`
2. Adding a case in `resolve_kind()` in the same file
3. An implementation module in `crates/ralph/src/strategy/`
4. A dispatch arm in `execute.rs`

## Error Handling

All errors include the file path and specific field/issue:
- Malformed TOML → parse error with path
- Unknown `kind` → lists known kinds
- Missing persona → names the missing persona
- Empty `prompt_aggregates` entry → reports index

Unknown strategy name in `ralph strategy execute` lists all available strategies.
