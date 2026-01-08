# Configuration

Ralph uses a TOML configuration file for persistent settings. This document covers file locations, available options, and configuration precedence.

## Configuration File Location

Ralph stores its configuration in platform-specific directories:

| Platform | Config Directory | Config File |
|----------|------------------|-------------|
| **Linux** | `~/.config/ralph/` | `~/.config/ralph/config.toml` |
| **macOS** | `~/Library/Application Support/ralph/` | `~/Library/Application Support/ralph/config.toml` |
| **Windows** | `%APPDATA%\ralph\` | `%APPDATA%\ralph\config.toml` |

### Environment Variable Overrides

You can override the default directories using environment variables:

| Variable | Purpose |
|----------|---------|
| `RALPH_CONFIG_DIR` | Override config directory (where `config.toml` is stored) |
| `RALPH_DATA_DIR` | Override data directory (where sessions are stored) |

Example:
```bash
# Use a custom config directory
export RALPH_CONFIG_DIR=/path/to/custom/config
ralph run

# Use a different data directory for sessions
export RALPH_DATA_DIR=/mnt/large-disk/ralph-sessions
ralph run
```

## Configuration Precedence

Settings are resolved in this order (highest priority first):

1. **CLI flags** (e.g., `--theme "Solarized (dark)"`)
2. **Environment variables** (e.g., `RALPH_THEME=Solarized (dark)`)
3. **Config file** (`config.toml` in config directory)
4. **Default values**

This means CLI flags always win, allowing you to override config file settings for specific invocations.

## Configuration File Format

The config file uses TOML format. All sections and fields are optional - only include what you want to customize.

### Complete Example

```toml
# Ralph Configuration File
# Location: ~/.config/ralph/config.toml (Linux)
#           ~/Library/Application Support/ralph/config.toml (macOS)
#           %APPDATA%\ralph\config.toml (Windows)

[theme]
# Syntax highlighting theme for code blocks and diffs
# Can be a built-in theme name or path to a .tmTheme file
name = "base16-ocean.dark"

# When true, don't apply theme background colors
# Useful if you want your terminal's background to show through
no_background = false
```

### Minimal Example

```toml
# Just set the theme, everything else uses defaults
[theme]
name = "Solarized (dark)"
```

## Configuration Sections

### `[theme]` - Syntax Highlighting

Controls how code blocks and diffs are displayed in the terminal.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `name` | string | `"base16-ocean.dark"` | Theme name or path to `.tmTheme` file |
| `no_background` | bool | `false` | Disable theme background colors |

#### Built-in Themes

Run `ralph themes` to see all available themes:

```
Available syntax highlighting themes:

  InspiredGitHub
  Solarized (dark)
  Solarized (light)
  base16-eighties.dark
  base16-mocha.dark
  base16-ocean.dark (default)
  base16-ocean.light
```

#### Using a Custom Theme

You can use any TextMate-compatible `.tmTheme` file:

```toml
[theme]
name = "/path/to/my-custom-theme.tmTheme"
```

#### Theme Environment Variable

The `RALPH_THEME` environment variable can be used instead of the config file:

```bash
# Set theme via environment variable
export RALPH_THEME="Solarized (dark)"
ralph run

# Or for a single command
RALPH_THEME="Monokai Extended" ralph run
```

#### Disabling Background Colors

If your terminal has a specific background color you want to preserve:

```toml
[theme]
name = "base16-ocean.dark"
no_background = true
```

Or via environment variable:
```bash
export RALPH_NO_BACKGROUND=1
```

Or via CLI flag:
```bash
ralph run --no-background
```

## Data Directory Structure

Ralph stores session data separately from configuration:

| Platform | Data Directory |
|----------|----------------|
| **Linux** | `~/.local/share/ralph/` |
| **macOS** | `~/Library/Application Support/ralph/` |
| **Windows** | `%APPDATA%\ralph\` |

Contents:
```
~/.local/share/ralph/           # Data directory
├── sessions.toml               # Session index
└── sessions/                   # Session directories
    ├── quiet-mountain/         # One directory per session
    │   ├── session.toml        # Session metadata
    │   ├── iteration-1.toml    # First iteration log
    │   ├── iteration-2.toml    # Second iteration log
    │   └── ...
    └── bright-river/
        └── ...
```

## Verbose Tools Configuration

The `--verbose-tools` flag controls how much detail is shown for LLM tool invocations during `ralph run`.

### Usage

```bash
# Enable verbose output for all tools
ralph run --verbose-tools

# Enable verbose for specific tools (comma-separated, case-insensitive)
ralph run --verbose-tools=read,grep,bash

# Single tool
ralph run --verbose-tools=glob
```

### Supported Tools

| Tool | Verbose Output Shows |
|------|---------------------|
| `read` | Full file content with syntax highlighting and line numbers |
| `edit` | Full diff with syntax highlighting |
| `write` | Full diff showing new/changed content |
| `glob` | All matched files grouped by directory |
| `grep` | Full regex pattern and matched content |
| `bash` | Full command and output with shell syntax highlighting |
| `todowrite` | Full todo list with status indicators |
| `notebookedit` | Full cell diff for Jupyter notebooks |

### Non-Verbose Mode (Default)

Without `--verbose-tools`, tool output is shown in a compact format:
- File paths shown without content
- Large outputs truncated
- Pattern matches summarized

## Environment Variables Reference

| Variable | Purpose | Example |
|----------|---------|---------|
| `RALPH_CONFIG_DIR` | Override config directory | `/custom/config` |
| `RALPH_DATA_DIR` | Override data directory | `/mnt/data/ralph` |
| `RALPH_THEME` | Set syntax highlighting theme | `Solarized (dark)` |
| `RALPH_NO_BACKGROUND` | Disable theme backgrounds | `1` |

## Troubleshooting

### Finding Your Config Location

If you're unsure where ralph is looking for configuration:

```bash
# Check what paths ralph uses
ralph --version
# Then check the platform-specific path for your OS
```

### Config File Not Loading

1. Verify the file exists at the correct path
2. Check file permissions (must be readable)
3. Validate TOML syntax - even small errors prevent loading:
   ```bash
   # Test TOML validity
   cat ~/.config/ralph/config.toml | python3 -c "import sys, tomllib; tomllib.loads(sys.stdin.read())"
   ```

### Theme Not Applying

1. Verify the theme name matches exactly (run `ralph themes` for the list)
2. For custom themes, ensure the file path is absolute and the file exists
3. Check if a CLI flag or environment variable is overriding your config
