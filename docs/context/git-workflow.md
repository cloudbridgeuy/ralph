# Git Workflow and Progressive Disclosure

This document describes ralph's git workflow conventions and the Progressive Disclosure documentation pattern. These practices replace the previous design.md and progress.txt files with a simpler, more maintainable approach.

## Overview

Ralph uses a simplified context model:
- **Git history** provides historical context (replaces progress.txt)
- **Progressive Disclosure** provides architectural context (replaces design.md)
- **PRD** is the only required context file for iteration-driven work

## Semantic Commits

All commits should follow the conventional commit format. This makes git history self-documenting and enables tools to parse commit semantics.

### Format

```
<type>(<scope>): <description>

[optional body]

[optional footer]
```

### Types

| Type | Description |
|------|-------------|
| `feat` | New feature or functionality |
| `fix` | Bug fix |
| `refactor` | Code restructuring without behavior change |
| `docs` | Documentation only |
| `test` | Adding or updating tests |
| `chore` | Maintenance tasks (deps, configs, etc.) |
| `perf` | Performance improvement |
| `style` | Formatting, whitespace (no logic change) |

### Examples

```bash
# New feature
feat(ask): add --clone-session flag for branching conversations

# Bug fix
fix(session): prevent duplicate iteration sequence numbers

# Refactoring
refactor(startup): extract display functions to separate module

# Documentation
docs: add git-workflow.md for Progressive Disclosure pattern

# Multiple scopes
feat(cli,ask): add --history flag with terminal rendering
```

### Guidelines

1. **Describe the "why"**, not just the "what"
2. **Keep the first line under 72 characters**
3. **Use imperative mood**: "add feature" not "added feature"
4. **No Claude attribution** in commit messages
5. **Scope is optional** but helps with filtering history

## Using Git History for Context

When starting work on a codebase, use git history to understand recent changes:

```bash
# View recent commits with one-line summaries
git log --oneline -10

# View commits affecting a specific file
git log --oneline -5 -- src/main.rs

# View commits with full messages for more context
git log -5

# View recent changes with diffs
git log -p -3
```

### What to Look For

1. **Commit style**: Match the existing commit message format
2. **Recent focus areas**: What parts of the codebase are actively changing
3. **Related changes**: Previous commits that touch the same files you're modifying
4. **Patterns**: How similar features were implemented before

## Progressive Disclosure Pattern

Documentation is organized in layers, from high-level overview to detailed specifics. This pattern allows readers (both humans and AI agents) to quickly find relevant information without wading through unnecessary detail.

### Layer 1: CLAUDE.md

The root `CLAUDE.md` file contains:
- Project conventions and unnegotiables
- Quick reference commands
- Links to detailed context files
- Glossary of project-specific terms

**Purpose**: Provide everything needed for basic orientation and simple tasks.

### Layer 2: docs/context/*.md

Detailed documentation for specific topics:
- CLI patterns and conventions
- Error handling approaches
- Serialization patterns
- Architecture decisions

**Purpose**: Deep dive into specific areas when needed.

### Document Structure

```
project/
├── CLAUDE.md                    # Layer 1: Overview + links
└── docs/
    └── context/
        ├── rust-cli-clap.md     # Layer 2: CLI patterns
        ├── rust-cli-error-handling.md
        ├── rust-cli-serde.md
        ├── stream-processor-architecture.md
        ├── prompt-template.md
        └── git-workflow.md
```

### When to Update Documentation

| Change Type | Update |
|-------------|--------|
| New project convention | CLAUDE.md |
| New architectural pattern | docs/context/*.md |
| New CLI feature | Existing relevant context doc |
| Major refactoring | Both if it changes conventions |
| Bug fix | Usually none |

### Writing Guidelines

1. **Don't duplicate information** - Link between documents instead
2. **Keep CLAUDE.md concise** - Move details to context files
3. **Use tables and code blocks** - Easier to scan than prose
4. **Include code locations** - Help readers find implementations
5. **Prefer examples over explanations** - Show, don't just tell

## docs/context/ vs docs/

The `docs/context/` directory is specifically for AI agent consumption. It uses patterns and formats optimized for LLM comprehension:
- Structured tables over flowing prose
- Code examples showing exact patterns
- Explicit "when to use" sections
- Code location references

The top-level `docs/` directory (if present) is for human documentation like user guides, API references, and tutorials.

## Two-Phase Workflow

Ralph's default prompt template enforces a two-phase workflow. For prompt customization details, see [Prompt Template System](prompt-template.md).

### Phase 1: Work

1. Understand the task from the PRD
2. Check recent git history for context
3. Consult CLAUDE.md and relevant docs/context/*.md
4. Implement the feature
5. Run `cargo xtask lint` to verify

### Phase 2: Commit

1. Update documentation per Progressive Disclosure:
   - Does this change project conventions? → Update CLAUDE.md
   - Does this add detailed context? → Create/update docs/context/*.md
   - Link new docs from CLAUDE.md
2. Make a semantic commit without Claude attribution

### Why Two Phases?

Separating work from documentation ensures:
- Documentation reflects what was actually implemented (not the plan)
- Commits are atomic and well-described
- Documentation stays up-to-date with the code

## Code Locations

| Component | File | Description |
|-----------|------|-------------|
| `PROMPT_TEMPLATE` | `crates/core/src/context.rs` | Default template with two-phase workflow |
| `defaults::PRD_FILE` | `crates/core/src/context.rs` | Default PRD path |
| Context retrieval instructions | In PROMPT_TEMPLATE | Git log and Progressive Disclosure usage |

## Summary

1. **Use semantic commits** - Format: `type(scope): description`
2. **Check git history** - `git log --oneline -10` before starting work
3. **Follow Progressive Disclosure** - CLAUDE.md → docs/context/*.md
4. **Separate work from documentation** - Two-phase workflow
5. **Keep docs/context/ for AI** - Structured, scannable content
