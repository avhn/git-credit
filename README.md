# git-credit

[![CI](https://github.com/avhn/git-credit/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/avhn/git-credit/actions/workflows/ci.yml)
[![Dependency audit](https://github.com/avhn/git-credit/actions/workflows/audit.yml/badge.svg)](https://github.com/avhn/git-credit/actions/workflows/audit.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

Fair git authorship stats. `git-credit` reports how much of a repository each person
wrote and how much work they put in, without letting a reformat, a renamed variable
or a one-character fix take the credit away from the original author.

> **Status: early development.** The pure line-attribution engine, repository
> scaffold and CI are in place. Repository replay and the `analyze` command are
> not implemented yet (see [Roadmap](#roadmap)). Nothing here is released yet.

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
- **Identities merge** through `.mailmap`, and `Co-authored-by` trailers split credit.
- **Noise is excluded:** lockfiles, generated and vendored code, binaries and
  configurable paths.

## Planned usage

```console
$ git-credit analyze
AUTHOR          OWNERSHIP   LABOR
Alice           54.2%       12,840
Bob             31.0%        9,115
Carol           14.8%        6,402

$ git-credit analyze --json > credit.json
$ git-credit badge --style bar --out credit.svg
```

A reusable GitHub Action will run on every push to `main` and publish badges to a
`badges` branch for your README.

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
| 1     | `analyze`: line-level ownership and labor, similarity threshold, formatting detection, renames, mailmap, co-authors, JSON | In progress |
| 2     | Token-level attribution with tree-sitter, merge-aware history, bulk-edit detection        | Planned     |
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
