# ralph strategy sync

Scaffolds a project with bundled agent definitions, strategy files, and a team strategy file. Idempotent — safe to re-run at any time to pick up new bundled assets.

## Code Locations

| Component | File | Description |
|-----------|------|-------------|
| CLI variant | `crates/ralph/src/cli/mod.rs` | `StrategyAction::Sync` |
| CLI test | `crates/ralph/src/cli/tests/strategy.rs` | `test_strategy_sync_parses` |
| Pure planner (FC) | `crates/ralph/src/strategy/init.rs` | `plan_file_actions()`, `plan_strategy_toml_action()`, `format_action()`, `format_sync_summary()` |
| Imperative shell (IS) | `crates/ralph/src/strategy/init.rs` | `execute_sync()`, `check_existing()`, `write_planned_files()`, `create_dir()` |
| Bundled assets | `crates/ralph/src/strategy/assets.rs` | `AGENT_ASSETS`, `STRATEGY_ASSETS`, `STRATEGY_TOML` via `include_str!` |
| Source agents | `assets/agents/*.md` | Markdown agent files compiled into the binary |
| Source strategies | `assets/strategies/*.toml` | Strategy TOML files compiled into the binary |
| Source template | `assets/strategy.toml` | Default `.claude/strategy.toml` template |
| Main wiring | `crates/ralph/src/main.rs` | `execute_strategy_sync()` |

## What It Creates

```
.claude/
  strategy.toml            ← team roster (always overwritten)
  agents/
    architect.md           ← created if missing
    developer.md
    reviewer.md
    tester.md
    product-manager.md
    storyteller.md
    editor-agent.md
    worldbuilder.md
    critic.md
  strategies/
    prd-loop.toml          ← created if missing
    fiction-loop.toml
```

## Idempotent Behavior

| File type | Exists? | Action |
|-----------|---------|--------|
| Agent `.md` | no | Created |
| Agent `.md` | yes | Skipped (user owns it) |
| Strategy `.toml` | no | Created |
| Strategy `.toml` | yes | Skipped (user owns it) |
| `strategy.toml` | no | Created |
| `strategy.toml` | yes | **Overwritten** (always synced to latest) |

Agent and strategy files are user-owned — once created, they are never overwritten. Only `strategy.toml` (the team roster) is kept in sync with the bundled template so that new agents added to ralph are automatically picked up.

## Architecture (FC-IS)

### Functional Core (pure, unit-tested)

- `plan_file_actions(target_dir, assets, existing_flags) -> Vec<InitAction>` — decides Create/Skip per file
- `plan_strategy_toml_action(path, exists) -> InitAction` — decides Create/Overwrite for `strategy.toml`
- `format_action(action) -> String` — renders one action as a summary line
- `format_sync_summary(agents, strategies, toml_action) -> String` — renders the full summary

### Imperative Shell (I/O)

- `execute_sync(project_path)` — orchestrates the full workflow:
  1. Create `.claude/agents/` and `.claude/strategies/` directories
  2. Gather file-existence flags via `check_existing()`
  3. Plan actions (pure)
  4. Write files via `write_planned_files()`
  5. Always overwrite `strategy.toml`
  6. Print summary

### Types

| Type | Purpose |
|------|---------|
| `InitAction` | `Created { path }`, `Skipped { path }`, `Overwritten { path }` |
| `InitError` | `WriteFile { path, source }`, `CreateDir { path, source }` |

## Bundled Assets

Assets live in `assets/` in the source tree and are compiled into the binary via `include_str!` in `crates/ralph/src/strategy/assets.rs`.

To add a new bundled agent:
1. Create `assets/agents/<name>.md` with YAML frontmatter (`name`, `description`)
2. Add an `include_str!` entry to `AGENT_ASSETS` in `assets.rs`
3. Add the agent to `assets/strategy.toml` `[agents]` table
4. Update `test_agent_assets_count` in `init.rs`

To add a new bundled strategy:
1. Create `assets/strategies/<name>.toml`
2. Add an `include_str!` entry to `STRATEGY_ASSETS` in `assets.rs`
3. Update `test_strategy_assets_count` in `init.rs`
