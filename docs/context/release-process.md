# Release Process

Releases publish macOS binaries to GitHub Releases. There is no Homebrew formula and no crates.io publish.

## Cut a release

1. Switch to the branch to release from, with a clean working tree.
2. Run `cargo xtask release <x.y.z>`.
3. Run the printed command: `git push origin <branch> vX.Y.Z`.

The command refuses a dirty tree, an existing tag, and a version that is not greater than the current one. It rewrites `[workspace.package] version` in `Cargo.toml`, refreshes `Cargo.lock`, commits `chore(release): vX.Y.Z`, and creates the annotated tag `vX.Y.Z`. It never pushes.

## What CI does

`.github/workflows/release.yml` runs on every pushed `v*` tag.

1. Fails unless the tagged commit is on `main` and the tag equals `v` plus the workspace version.
2. Builds `ralph` with `--release --locked` for `aarch64-apple-darwin` and `x86_64-apple-darwin`.
3. Packages `ralph-vX.Y.Z-<target>.tar.gz` with a `.sha256` file.
4. Creates the GitHub Release and attaches all assets.

## Version scheme

All crates share one version, set in `[workspace.package]`. `ralph --version` (or `-V`) reports:

| Build | Output |
| --- | --- |
| Clean checkout at tag `v<cargo version>` | `<cargo version>` |
| Any other build | `<next>-dev+<sha>`, plus `.dirty` when the tree has changes |
| No git information | `<next>-dev` |

`<next>` is the Cargo version with the patch number raised by one.
