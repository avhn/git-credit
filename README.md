# git-credit

[![CI](https://github.com/avhn/git-credit/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/avhn/git-credit/actions/workflows/ci.yml)
[![Dependency audit](https://github.com/avhn/git-credit/actions/workflows/audit.yml/badge.svg)](https://github.com/avhn/git-credit/actions/workflows/audit.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

Fair git authorship stats. `git-credit` reports how much of a repository each person
wrote and how much work they put in, without letting a reformat, a renamed variable
or a one-character fix take the credit away from the original author.

> **Status: line-level analysis available.** `analyze` reads committed HEAD and
> reports ownership and labor using merge-aware full-history replay. Branch authors
> keep credit through merges; token-level analysis is planned. These metrics
> describe provenance, not a person's value.

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

- **Formatting is free within the normalization rules.** Trailing whitespace and
  CRLF changes earn no credit and take none away. Leading indentation is ignored
  only when the language permits it. Interior whitespace and line wrapping can
  still be meaningful changes in the line-level engine.
- **Small edits keep the original author.** A line that changes only slightly stays
  with its author; the editor gets labor for the change, not ownership of the line.
  The similarity threshold is configurable. Later versions attribute individual tokens.
- **Moved and renamed code keeps its author.**
- **Merges preserve branch provenance.** Each reachable commit contributes labor
  once; inherited parent content does not earn the merge author new labor.
- **Identities merge** through `.mailmap`, and `Co-authored-by` trailers split credit.
- **Noise is excluded:** lockfiles, generated and vendored code, binaries and
  configurable paths.

## Usage

```console
$ cargo run --locked -- analyze .
$ cargo run --locked -- analyze ../another-repository --json > credit.json
$ cargo run --locked -- analyze . --keep 0.80 --exclude 'generated/**' --depth 2
$ cargo install --path crates/git-credit --locked
$ git-credit analyze --quiet --no-color
```

The table shows `AUTHOR`, `OWNERSHIP %`, `OWNED LINES` and `LABOR`, sorted by
ownership descending. The footer identifies HEAD, commit count, walk mode and
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
Full mode matches exact blobs and normalized inserted or rewritten lines from
merge parents; unmatched lines are merge-resolution work.

### Configuration

Optional `.git-credit.toml` at the repository root:

```toml
walk = "full" # Use "first-parent" for legacy merge attribution.
keep = 0.70
group_by = "name" # Use "email" to keep different addresses separate.
exclude = ["generated/**", "fixtures/**"]
opt_out = ["private@example.test"]
max_file_size = 1048576
```

Explicit `--keep` overrides config; `--first-parent` overrides the configured walk. Repeated `--exclude` flags replace config
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
CLI usage. Badges and a reusable GitHub Action remain planned.

## Prerequisites

- Rust stable via [rustup](https://rustup.rs). `rust-toolchain.toml` selects the
  channel and components; the minimum supported Rust version is the `rust-version`
  in `Cargo.toml`.
- Git, for running the tool and its tests.

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
  git-credit/    the command-line binary
docs/PLAN.md     architecture and roadmap
tools/           repository policy scripts and their tests
```

## Roadmap

| Phase | Scope                                                                                     | State       |
| ----- | ----------------------------------------------------------------------------------------- | ----------- |
| 0     | Workspace, CI on Linux, Windows and macOS, policy checks, licensing                       | Done        |
| 1     | `analyze`: line-level ownership and labor, similarity threshold, formatting detection, renames, mailmap, co-authors, JSON | Done        |
| 2     | Token-level attribution with tree-sitter, merge-aware history, bulk-edit detection        | Merges done; tokens and bulk edits planned |
| 3     | Incremental cache, fuzzy move detection, benchmarks                                       | Planned     |
| 4     | SVG badges and cards, shields.io endpoint, reusable GitHub Action                         | Planned     |

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
