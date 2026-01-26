# Prompt Template System

This document describes how ralph's prompt template system works, including available placeholders, customization options, and the two-phase workflow enforced by the default template.

## Overview

Ralph uses a prompt template system to construct the instructions sent to the LLM. The template supports placeholder substitution and can be customized via CLI flags, file paths, or stdin.

## Code Location

| Component | File | Description |
|-----------|------|-------------|
| `defaults::PROMPT_TEMPLATE` | `crates/core/src/context.rs` | Default template constant |
| `defaults::PRD_FILE` | `crates/core/src/context.rs` | Default PRD path |
| `defaults::COMPLETION_MARKER` | `crates/core/src/context.rs` | Default completion marker |
| `substitute_template_placeholders` | `crates/core/src/context.rs` | Placeholder substitution function |
| `resolve_prompt` | `crates/ralph/src/main.rs` | Prompt resolution with source detection |
| `classify_prompt_source` | `crates/ralph/src/main.rs` | Input type classification |
| `PromptSource` | `crates/ralph/src/main.rs` | Source type enum |
| `--prompt`, `--prd`, `--completion-marker` | `crates/ralph/src/cli/mod.rs` | CLI flag definitions |

## Available Placeholders

| Placeholder | Description | Default Value |
|-------------|-------------|---------------|
| `{prd_file}` | Path to the PRD file | `.local/plans/prd.toml` |
| `{completion_marker}` | String to output when all stories complete | `<promise>COMPLETE</promise>` |
| `{additional_prompt}` | Extra instructions appended to the prompt | Empty string |

Default values are defined in `crates/core/src/context.rs` in the `defaults` module.

### Example Substitution

```
Template: @{prd_file}
Result:   @/Users/me/project/.local/plans/prd.toml
```

The `@` prefix is Claude CLI syntax for including file contents in the context.

## Default Template Behavior

The default template follows the simplified context model:

- Only the PRD file is referenced as context (no design.md or progress.txt)
- Git history provides historical context (`git log --oneline`)
- Progressive Disclosure pattern provides architectural context (CLAUDE.md + docs/context/*.md)
- Two-phase workflow: work first, then document and commit

The actual template content is in `PROMPT_TEMPLATE` in `crates/core/src/context.rs`.

## CLI Customization

### Using `--prompt` Flag

The `--prompt` flag accepts three input types:

```bash
# Inline prompt (literal string)
ralph run --prompt "Custom instructions here"

# File path (reads content from file)
ralph run --prompt /path/to/custom-prompt.txt

# Stdin (use "-" to read from stdin)
cat custom-prompt.txt | ralph run --prompt -
```

### Input Classification Logic

1. If the value is "-", read from stdin
2. If the value is a path to an existing file, read from that file
3. Otherwise, treat the value as an inline string

### Using `--additional-prompt` Flag

Append extra instructions to the default template without replacing it:

```bash
# Inline additional instructions
ralph run --additional-prompt "Focus on error handling"

# From file
ralph run --additional-prompt /path/to/extra-instructions.txt

# From stdin
echo "Focus on tests" | ralph run --additional-prompt -
```

The additional prompt is substituted into `{additional_prompt}` in the template.

### Overriding Defaults

```bash
# Custom PRD path
ralph run --prd /path/to/custom/prd.toml

# Custom completion marker
ralph run --completion-marker "<done/>"
```

## Custom Prompt Templates

To use a completely custom prompt template, create a file with your template and pass it via `--prompt`:

```bash
ralph run --prompt /path/to/my-templates/custom-prompt.txt
```

### Template File Example

Use a custom template for non-PRD-driven workflows or specialized domains:

```
@{prd_file}

## Instructions

1. Review the PRD stories
2. Implement the next pending story
3. Output {completion_marker} when done

{additional_prompt}
```

All standard placeholders work in custom templates.

## Configuration Precedence

For prompt-related settings, the precedence is:

1. **CLI arguments** - `--prompt`, `--prd`, `--completion-marker`
2. **Default values** - Built-in template and defaults

**Note:** Unlike theme settings, prompt configuration does not support environment variables or config file overrides. This keeps prompt customization explicit and visible in the command invocation.

## Best Practices

### When to Use Custom Prompts

- Project-specific workflows that differ from the default two-phase approach
- Non-PRD-driven work (e.g., one-off tasks, exploration)
- Specialized domains requiring different instructions

### When to Use Additional Prompts

- Adding project-specific constraints to the default workflow
- Emphasizing particular quality attributes (performance, security, etc.)
- Including one-off context for a specific run

### Template Design Guidelines

1. **Include file references with `@`** - Claude CLI uses `@` to read file contents
2. **Use `{completion_marker}`** - Allows ralph to detect when the LLM considers work complete
3. **Keep instructions actionable** - Clear steps the LLM can follow
4. **Separate phases** - Distinguish between work and commit/documentation phases
