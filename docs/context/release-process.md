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
5. Unless `--no-summary` is given, `claude -p --model claude-haiku-5-5` writes a short summary paragraph from the list. If it fails, the command warns and continues without it.
6. The result opens in `$VISUAL`, `$EDITOR` or `vi`. Empty notes abort the release before any file is written.

The final text is prepended to `CHANGELOG.md` under `## vX.Y.Z - YYYY-MM-DD`, newest first.

## What CI does

`.github/workflows/release.yml` runs on every pushed `v*` tag.

All workflows pin Linux runners to `ubuntu-26.04` instead of `ubuntu-latest`, so GitHub image moves never change CI without an explicit bump.

1. Fails unless the tagged commit is on `main` and the tag equals `v` plus the workspace version.
2. Fails unless `CHANGELOG.md` has a non-empty `## vX.Y.Z` section. That section is the release body.
3. Builds `ralph` with `--release --locked` for `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`.
4. Packages `ralph-vX.Y.Z-<target>.tar.gz` with a `.sha256` file.
5. Creates the GitHub Release with `--notes-file` and attaches all assets. The notes file is not an asset.

Linux binaries are built on `ubuntu-26.04` and link glibc dynamically. They need that release's glibc or newer; older distributions may fail to start them.

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
