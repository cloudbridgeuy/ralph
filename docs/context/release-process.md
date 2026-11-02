# Release Process

Releases publish macOS and Linux binaries to GitHub Releases. There is no Homebrew formula and no crates.io publish.

## Cut a release

1. Switch to the branch to release from, with a clean working tree.
2. Run `cargo xtask release <x.y.z>`. Your editor opens on the generated release notes; edit and save them.
3. Run the printed command: `git push origin <branch> vX.Y.Z`.

The command refuses a dirty tree, an existing tag, and a version that is not greater than the current one. It rewrites `[workspace.package] version` in `Cargo.toml`, refreshes `Cargo.lock`, prepends the notes to `CHANGELOG.md`, commits `chore(release): vX.Y.Z`, and creates the annotated tag `vX.Y.Z`. It never pushes.

Flags:

- `--dry-run`: print the generated section to stdout and stop. No editor, no file writes, no commit, no tag. A dirty tree is allowed.
- `--no-summary`: skip the summary paragraph.

## Release notes

`cargo xtask release` builds the notes from `git log <previous v* tag>..HEAD --no-merges`.

1. Commits are grouped by the `GUZ-<n>` ids in their subject and body. Each ticket becomes one entry: `- <title> (GUZ-<n>)`, sorted by id.
2. If `LINEAR_API_KEY` is set in the environment, titles and labels come from the Linear API (50 tickets per request). The key is read from the environment only. A `Feature`, `Improvement` or `Bug` label picks `Features`, `Improvements` or `Fixes`.
3. Without the key, after any lookup failure, or when a ticket has no mapped label, the command prints a warning and uses the first commit instead: `feat` goes to `Features`, `fix` to `Fixes`, anything else to `Improvements`. The entry text is the commit subject.
4. Commits without a ticket go to `Other`: `feat` and `fix` subjects are listed, the rest become `- plus N other commits`.
5. Unless `--no-summary` is given, `claude -p --model claude-haiku-5-5 --safe-mode --tools "" --no-session-persistence --system-prompt <...>` writes a short summary paragraph from the list. Safe mode skips CLAUDE.md, skills, plugins, hooks and MCP servers, so user rules do not shape the text. The command then removes code fence lines and a short leading line such as "Here's the summary:" (six words or fewer, ending in `:`). After the first blank line it also removes trailing chatter: lines that start with `next:`, `note:` or `let me know`, lines that contain `paste this` or `changelog entry above`, and short questions that stand as their own paragraph. The first paragraph is never cut. If `claude` fails, returns no text, or returns only chatter, the command warns `Summary skipped (...)` and continues without a summary.
6. The result opens in `$VISUAL`, `$EDITOR` or `vi`. Empty notes abort the release before any file is written.

The final text is prepended to `CHANGELOG.md` under `## vX.Y.Z - YYYY-MM-DD`, newest first.

## What CI does

`.github/workflows/release.yml` runs on every pushed `v*` tag.

All workflows pin Linux runners to `ubuntu-26.04` instead of `ubuntu-latest`, so GitHub image moves never change CI without an explicit bump.

1. Fails unless the tagged commit is on `main` and the tag equals `v` plus the workspace version.
2. Fails unless `CHANGELOG.md` has a non-empty `## vX.Y.Z` section. That section is the release body.
3. Builds `ralph` with `--release --locked` for `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`.
4. Packages `ralph-vX.Y.Z-<target>.tar.gz` with a `.sha256` file. See [Asset contract](#asset-contract).
5. Creates the GitHub Release with `--notes-file` and attaches all assets. The notes file is not an asset.

Linux binaries are built on `ubuntu-26.04` and link glibc dynamically. They need that release's glibc or newer; older distributions may fail to start them.

## Asset contract

`ralph upgrade` depends on the asset layout that the `Package` step of `release.yml` writes. Change both together: the workflow and `crates/core/src/upgrade.rs` (`pick_download`, `verify_checksum`, `extract_binary`, `target_triple`).

- Archive: `ralph-vX.Y.Z-<target>.tar.gz`, where the tag is `v` plus the workspace version.
- The archive holds one regular file, `ralph`, at its root.
- Checksum: `ralph-vX.Y.Z-<target>.tar.gz.sha256`, one `shasum -a 256` line: `<64 hex>  <archive name>`.
- Targets: `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`.

v0.2.0 has only the two macOS targets. Linux assets start with the first release after it.

## Upgrade an install

`ralph upgrade` replaces the running binary with the latest published release:

1. Gets `https://api.github.com/repos/cloudbridgeuy/ralph/releases` without a token. Drafts, prereleases and tags that are not `vX.Y.Z` are ignored. The highest version is the latest.
2. Compares it with `ralph --version`. If the latest is not newer, it prints that ralph is up to date and exits 0.
3. Refuses a dev build (`-dev`, installed from source) unless `--force`. With `--force` it installs the latest release even when that is older than the dev build.
4. Picks the asset for the build's arch and OS. Other platforms get an error.
5. Resolves the real path of the binary (symlinks followed) and creates a temp file in its directory. If the directory is not writable, it stops with the path before it downloads. For `/usr/local/bin` this usually means `sudo ralph upgrade`.
6. Downloads the archive and its `.sha256`, checks the digest, extracts `ralph` in memory, sets mode `755`, and renames the temp file over the binary.
7. Prints the release notes of every skipped release, newest first.

`--check` stops after step 3, prints the result and the notes, and downloads nothing. Without network, the command fails with `cannot reach GitHub` and changes nothing. Running chats and workers keep the old binary until they restart.

The first release with `ralph upgrade` must be installed by hand; v0.2.0 does not have the command.

## Test run without a release

Run the `Release` workflow manually from the Actions tab (`workflow_dispatch`). It skips the tag checks and the publish job, builds all four targets, and uploads one workflow artifact per target (`ralph-<target>`) holding `ralph-dev-<short sha>-<target>.tar.gz` and its `.sha256`. The optional `version` input (for example `v0.3.0`) also checks that `CHANGELOG.md` has that section; leave it empty to skip the check.

## Version scheme

All crates share one version, set in `[workspace.package]`. `ralph --version` (or `-V`) reports:

| Build | Output |
| --- | --- |
| Clean checkout at tag `v<cargo version>` | `<cargo version>` |
| Any other build | `<next>-dev+<sha>`, plus `.dirty` when the tree has changes |
| No git information | `<next>-dev` |

`<next>` is the Cargo version with the patch number raised by one.
