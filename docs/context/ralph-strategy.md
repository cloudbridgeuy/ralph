# ralph strategy Command

This document describes the `ralph strategy` command, which manages and executes collaboration strategies — predefined patterns of persona interaction defined in TOML files.

## Code Locations

| Component | File | Description |
|-----------|------|-------------|
| CLI args | `crates/ralph/src/cli/mod.rs` | StrategyArgs, StrategyAction, StrategyExecuteArgs |
| Core types | `crates/core/src/strategy.rs` | StrategyConfig, StrategyError, parsing, validation |
| Discovery & display | `crates/ralph/src/strategy.rs` | File discovery, loading, validation, formatting |
| CLI tests | `crates/ralph/src/cli/tests/strategy.rs` | CLI argument parsing tests |
| Execution | `crates/ralph/src/main.rs` | execute_strategy(), execute_strategy_list(), execute_strategy_execute() |

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

## CLI Subcommands

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
- `validate_kind()` — check kind against known implementations
- `validate_personas()` — check persona references against discovered agents
- `validate_prompt_aggregates()` — reject empty string entries

**Imperative Shell** (`crates/ralph/src/strategy.rs`):
- `discover_strategies()` — scan `.claude/strategies/` directory for TOML files
- `load_and_validate_strategy()` — read file, parse, and run all validations
- `load_all_strategies()` — discover + load all strategies in project
- `find_strategy_by_name()` — lookup by name with actionable error for unknown names
- `format_strategy_list()` — columnar table output (pure, but lives in shell for display coupling)

## Known Kinds

Currently registered: `prd-loop`

New strategies require:
1. A Rust module implementing the Strategy trait (Story 3)
2. Registering its kind string in `KNOWN_KINDS`

## Error Handling

All errors include the file path and specific field/issue:
- Malformed TOML → parse error with path
- Unknown `kind` → lists known kinds
- Missing persona → names the missing persona
- Empty `prompt_aggregates` entry → reports index

Unknown strategy name in `ralph strategy execute` lists all available strategies.
