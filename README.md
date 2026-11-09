# git-credit

[![CI](https://github.com/avhn/git-credit/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/avhn/git-credit/actions/workflows/ci.yml)
[![Dependency audit](https://github.com/avhn/git-credit/actions/workflows/audit.yml/badge.svg)](https://github.com/avhn/git-credit/actions/workflows/audit.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

Fair git authorship stats. `git-credit` reports how much of a repository each person
wrote and how much work they put in, without letting a reformat, a renamed variable
or a one-character fix take the credit away from the original author.

> **Status: v0.1.1.** All planned phases are complete. `analyze` defaults to token-level attribution and
> merge-aware full-history replay. Reports stay in line-equivalent units. Use
> `--granularity line` for the legacy line rules. These metrics describe provenance,
> not a person's value.

## Why

`git blame` gives a whole line to whoever touched it last. Run a formatter over the
codebase and one person "owns" everything; fix a typo and the line is yours. Tools
built on blame inherit the problem, and work that was later rewritten or deleted
disappears from the numbers entirely.

`git-credit` reports two separate metrics:

| Metric        | Question it answers                                           | Changes when                                     |
| ------------- | ------------------------------------------------------------- | ------------------------------------------------ |
| **Ownership** | Who wrote the meaningful code that exists now?                | Code is genuinely rewritten or deleted           |
| **Labor**     | Who did the meaningful work over the whole history?           | Never decreases because of someone else's edits  |

Attribution rules:

- **Unchanged tokens keep their authors.** Only inserted and replaced tokens belong
  to the editor. Changing one of ten equal-weight tokens earns 0.1 line of labor,
  not ownership of the entire line. Every meaningful output line counts as one
  ownership unit, divided by its tokens' weights.
- **Formatting is free within the lexical rules.** Rewrapping, joining lines and
  adding a trailing comma earn no labor. String contents remain meaningful, as
  does indentation in Python and other indentation-sensitive files.
- **Bulk renames preserve provenance.** A dominant replacement mapping repeated
  at least five times retains token owners and earns one labor line per mapping
  per file. Reports identify discounted commits; `--no-bulk-discount` disables it.
- **Legacy line rules remain available.** `--granularity line` preserves the
  configurable similarity threshold and Phase 2a ownership/labor numbers.
- **Moved and renamed code keeps its author.**
- **Merges preserve branch provenance.** Each reachable commit contributes labor
  once; inherited parent content does not earn the merge author new labor.
- **Identities merge** through `.mailmap`, and `Co-authored-by` trailers split credit.
- **Noise is excluded:** lockfiles, generated and vendored code, binaries and
  configurable paths.

## Install

### Homebrew (macOS and Linux)

```sh
brew install avhn/tap/git-credit
```

### APT (Debian 11+ and Ubuntu 20.04+)

Packages support amd64 and arm64. Install the repository prerequisites, download
its public keyring, and verify the fingerprint against
`097DDFE92CE4B0F4E204F434668F8E1D756456FD` before trusting it.
The keyring must contain exactly one primary public key with that fingerprint:

```sh
sudo apt update && sudo apt install -y ca-certificates curl gnupg
curl -fsSL https://avhn.github.io/git-credit/apt/git-credit-archive-keyring.gpg -o git-credit-archive-keyring.gpg
gpg --show-keys --with-fingerprint git-credit-archive-keyring.gpg
sudo install -m 0644 git-credit-archive-keyring.gpg /usr/share/keyrings/git-credit-archive-keyring.gpg
echo 'deb [signed-by=/usr/share/keyrings/git-credit-archive-keyring.gpg] https://avhn.github.io/git-credit/apt stable main' | sudo tee /etc/apt/sources.list.d/git-credit.list
sudo apt update && sudo apt install git-credit
```

### Direct Debian package or binary

Download `git-credit_0.1.1-1_amd64.deb` (or `_arm64.deb`) and `SHA256SUMS` from the
[release](https://github.com/avhn/git-credit/releases/tag/v0.1.1). From that download
directory, verify the selected package and install it:

```sh
grep '  git-credit_0.1.1-1_amd64.deb$' SHA256SUMS | sha256sum -c -
sudo apt install ./git-credit_0.1.1-1_amd64.deb
```

Prebuilt archives for Linux (x86_64, ARM64), macOS (Intel, Apple Silicon) and Windows
(x86_64) remain attached to every [GitHub release](https://github.com/avhn/git-credit/releases)
with `SHA256SUMS`. Verify the selected archive's hash, then extract and place
`git-credit` (`git-credit.exe` on Windows) on your PATH. Linux archives use static
musl binaries, not a host glibc dependency. Checksums detect corruption, not a
compromised release account. See [packaging and signing](docs/packaging.md).

### Build from source

```console
$ cargo install --git https://github.com/avhn/git-credit --tag v0.1.1 --locked git-credit
```

## Usage

```console
$ cargo run --locked -- analyze .
$ cargo run --locked -- analyze ../another-repository --json > credit.json
$ cargo run --locked -- analyze . --granularity line --keep 0.80 --exclude 'generated/**' --depth 2
$ cargo install --path crates/git-credit --locked
$ git-credit analyze --quiet --no-color
```

The table shows `AUTHOR`, `OWNERSHIP %`, `OWNED LINES` and `LABOR`, sorted by
ownership descending. The footer identifies HEAD, commit count, walk mode, granularity and
algorithm version. JSON adds per-directory ownership and the effective policy;
see [the versioned contract](docs/json.md). Output is plain text, including when
piped. `--quiet` suppresses warnings on stderr, while `-v` enables diagnostics.
`NO_COLOR` and `--no-color` are respected because no color is emitted.

Only committed content is scored. Blank lines have zero weight. Shallow clones
fail with a `git fetch --unshallow` hint; analysis never fetches or runs hooks.
Bare repositories are supported. Symlinks, submodules, binaries and files over
1 MiB are skipped; the file-size limit is configurable. This build supports
SHA-1 repositories. Use `--first-parent` for the legacy chain-only analysis, which
credits new branch content in merge diffs to the merge author and co-authors.
Full mode matches exact blobs and inserted tokens (or normalized lines in line
mode) from merge parents; unmatched content is merge-resolution work. The lexer
is language-agnostic, not a syntax parser; quoted strings stop at physical line
ends and tabs use eight-column indentation stops.

### Incremental replay

Analysis caches replay state inside `.git/git-credit/cache/`, never the worktree.
Subsequent runs score only new commits when the required parent snapshots survive.
Cold and cached JSON are byte-identical. Use `--no-cache` to disable both reads and
writes, `--cache-dir DIR` to override the location, or `--cache-depth N` to retain
N first-parent ancestors in addition to HEAD (default 64). These flags also work
with `badge`. `-v` explains cache hits, rebuilds and the 256 MiB size bound.

Policy changes, history rewrites and merges forked beyond the retained depth cause
safe cold fallback. Identity changes that alter co-author splitting also rebuild;
current display identities and opt-out always apply. The cache contains historical
provenance and must not be published as badge output. See [performance measurements](docs/PERFORMANCE.md).

### Configuration

Optional `.git-credit.toml` at the repository root:

```toml
walk = "full" # Use "first-parent" for legacy merge attribution.
granularity = "token" # Use "line" for Phase 2a attribution.
bulk_discount = true
cache = true # Disable with false or --no-cache.
keep = 0.70 # Used only with line granularity.
group_by = "name" # Use "email" to keep different addresses separate.
exclude = ["generated/**", "fixtures/**"]
opt_out = ["private@example.test"]
max_file_size = 1048576
```

`--granularity` overrides config; `--no-bulk-discount` disables the configured
discount. Explicit `--keep` requires line granularity and overrides config there.
`--first-parent` overrides the configured walk. Repeated `--exclude` flags replace config
exclusions; built-in lockfile, vendor, node_modules, dist, minified and source-map
exclusions remain active. Thresholds are rounded to per-mille precision.
`--depth` defaults to 1; zero disables directory reporting. Names are grouped by
case-insensitive trimmed display name after mailmap resolution by default. The
most frequently used email represents each group, and JSON lists all merged
canonical addresses in `emails`. Use `--by-email` (overrides config) or
`group_by = "email"` when distinct people share a name. Matching any merged address
in `opt_out` anonymizes the entire group, including its earlier work, without
changing percentages or exposing its other addresses.
The committed HEAD `.mailmap` applies to all history, with configured
`mailmap.blob` and `mailmap.file` overrides. Worktree `.mailmap` edits are ignored.
Rename detection uses an isolated raw-object cache: staged/dirty attributes and
configured diff drivers or text conversion do not affect attribution.

Exit codes are 0 for success, 1 for analysis/configuration errors and 2 for invalid
CLI usage. Badges and the reusable GitHub Action are described below.

## Badges

![Contributions](https://raw.githubusercontent.com/avhn/git-credit/badges/credit-bar.svg)
![Contribution summary](https://raw.githubusercontent.com/avhn/git-credit/badges/credit-card.svg)
![Bus factor](https://raw.githubusercontent.com/avhn/git-credit/badges/bus-factor.svg)

```console
$ cargo run --locked -- badge . --out-dir .git-credit
$ git-credit badge --from credit.json --out-dir .git-credit --style card --theme dark
```

The source-built dogfood workflow publishes these files to an orphan `badges`
branch on main pushes. Images become available after its first successful run.
The reusable GitHub Action verifies release checksums and can publish to the same
kind of generated-only branch in other repositories. See [badge documentation](docs/badges.md)
for workflow examples, all inputs, shields.io endpoints and privacy/retention notes.
Opt-outs apply before rendering; imported reports must already be privacy-filtered.
The generated `credit.json` also contains non-opted-out canonical email addresses.

## Prerequisites

- Rust stable via [rustup](https://rustup.rs). `rust-toolchain.toml` selects the
  channel and components; the minimum supported Rust version is the `rust-version`
  in `Cargo.toml`.
- Git, for development and fixture tests. The installed CLI reads repositories
  through gix and does not require a Git executable.

## Setup and run

```console
$ git clone https://github.com/avhn/git-credit
$ cd git-credit
$ git config core.hooksPath .githooks   # commit message and formatting checks
$ cargo run -- --help
```

## Commands

| Command                                        | What it does                                        |
| ---------------------------------------------- | --------------------------------------------------- |
| `cargo build --workspace`                      | Build every crate                                   |
| `make analyze REPO=.`                          | Analyze committed HEAD as a table                   |
| `make json REPO=.`                             | Analyze committed HEAD as JSON                      |
| `cargo test --workspace`                       | Run all tests                                       |
| `cargo lint`                                   | Clippy with the same flags as CI (pedantic, deny)   |
| `cargo fmt --all`                              | Format the code                                     |
| `tools/check-text.sh`                          | Text policy: no en or em dashes, LF line endings    |
| `tools/tests/check-commit-msg.test.sh`         | Tests for the commit message checker                |

## Project layout

```text
crates/
  credit-core/   pure attribution engine: normalization, diff, ownership and labor ledgers (no I/O)
  credit-git/    repository access: history walk, renames, .mailmap, co-author trailers
  credit-badge/  pure deterministic SVG badges, cards and shields.io endpoint JSON
  git-credit/    the command-line binary
docs/PLAN.md     architecture and roadmap
tools/           repository policy scripts and their tests
```

## Roadmap

| Phase | Scope                                                                                     | State       |
| ----- | ----------------------------------------------------------------------------------------- | ----------- |
| 0     | Workspace, CI on Linux, Windows and macOS, policy checks, licensing                       | Done        |
| 1     | `analyze`: line-level ownership and labor, similarity threshold, formatting detection, renames, mailmap, co-authors, JSON | Done        |
| 2     | Language-agnostic token attribution, merge-aware history, bulk-edit detection             | Done |
| 3     | Incremental cache, fuzzy move detection, benchmarks                                       | Done     |
| 4     | SVG badges and cards, shields.io endpoints, reusable GitHub Action and tagged binaries   | Done |

Details and design decisions: [docs/PLAN.md](docs/PLAN.md).

## Contributing

Commits follow Conventional Commits (`type(scope): summary`, lowercase, no trailing
period); the `commit-msg` hook in `.githooks/` and CI enforce it. Before pushing, run
`cargo fmt --all`, `cargo lint` and `cargo test --workspace`.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for
inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual
licensed as above, without any additional terms or conditions.
