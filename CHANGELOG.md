# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Pure line-attribution engine with deterministic normalized diffs, bounded
  similarity pairing, fixed-point ownership and labor, co-author shares, and
  same-commit cross-file block moves. Algorithm version is now 1.
- Core tests for attribution rules, rounding, invalid inputs and ledger overflow.
  Repository replay and the `analyze` CLI remain unimplemented.

- Cargo workspace with `credit-core`, `credit-git` and the `git-credit` binary.
- CI on Linux, Windows and macOS: repository policy, secret scan, static checks,
  unit tests with coverage, builds including the minimum supported Rust version.
- Weekly dependency advisory audit and Dependabot updates.
- Dual licensing under MIT OR Apache-2.0.
