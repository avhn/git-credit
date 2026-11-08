# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Algorithm version 3 defaults to canonical token attribution with exactly conserved
  line-equivalent ownership and fractional insertion labor. Line granularity keeps
  Phase 2a credit numbers unchanged; `--keep` is restricted to line mode.
- Token moves and merge adoption preserve provenance and refund inherited work,
  including other-parent move sources with shared-ancestry deduplication.
- Algorithm version 2 defaults to merge-aware full-history replay. All reachable
  commits contribute labor once in deterministic parent-before-child order.
- Merge blobs and normalized editor lines matching other parents retain parent
  ownership without duplicate merge labor. Unmatched lines remain resolution work.
- Retained commit states share unchanged files and are released after their last
  reachable child. Verbose diagnostics include the peak retained state count.
- `--first-parent` and `walk = "first-parent"` retain legacy attribution. JSON `walk`
  now reports `full` or `first-parent`; schema version remains 1. Full mode no longer
  emits the approximate-merge warning.

### Added

- Language-agnostic token lexer, `--granularity line|token` and matching config,
  commit-wide bulk-rename detection, `--no-bulk-discount` / `bulk_discount = false`,
  JSON granularity and bulk counts, and warnings with up to five example commits.
- Exact-share, formatting, indentation, bulk-threshold, conservation and malformed
  input tests. Phase 2a merge fixtures now run under both granularities.
- Reproducible Myers/histogram token-stream benchmark; Myers remains the runtime
  choice. Histogram is a development-only comparison dependency.
- Dependency-free `credit-badge` renderer: escaped deterministic SVG contributor
  bars, light/dark summary cards, strict-majority bus-factor and safe per-author
  badges, plus shields.io endpoint JSON and complete percentage rounding.
- Offline `badge` command using the existing analysis/configuration/privacy path,
  schema 1 report import, style/top/width controls, silent stdout and stale managed
  asset cleanup after identity or style changes.
- Composite GitHub Action with release checksum verification, optional local binary,
  generated-only orphan publication, explicit leases and stale-source protection.
- Tagged release workflow for Linux x86_64/ARM64, macOS x86_64/ARM64 and Windows
  x86_64 archives, licenses, README and SHA256SUMS; source-built main badge dogfooding.
- Badge usage, action inputs and publication privacy/retention documentation.

- Pure line-attribution engine with deterministic normalized diffs, bounded
  similarity pairing, fixed-point ownership and labor, co-author shares, and
  same-commit cross-file block moves. Algorithm version is now 1.
- Offline gix first-parent replay with exact and similarity renames, committed HEAD
  mailmap resolution, co-author trailers, opt-out aggregation, directory ownership,
  binary/size exclusions, and shallow/missing-history errors.
- `analyze` table and schema-versioned JSON output, CLI/config precedence, built-in
  glob exclusions, quiet/verbose controls and pipe-clean output.
- Default mailmap-resolved name grouping with representative email frequency,
  merged `emails` in JSON, `--by-email` / `group_by = "email"` overrides, and
  group-wide opt-out redaction.
- Core and real Git fixture tests for attribution, configuration, malformed inputs,
  bare repositories, shallow clones and missing parent objects.

- Cargo workspace with `credit-core`, `credit-git` and the `git-credit` binary.
- CI on Linux, Windows and macOS: repository policy, secret scan, static checks,
  unit tests with coverage, builds including the minimum supported Rust version.
- Weekly dependency advisory audit and Dependabot updates.
- Dual licensing under MIT OR Apache-2.0.

### Fixed

- Bulk discounts now require identifier-to-identifier mappings. Repeated literal,
  keyword, operator, indentation and comment edits retain ordinary editor credit;
  their replacement weights still count toward the dominance denominator.

- Exact merge-parent blob adoption retains removed first-parent blocks as move
  sources. Moves can also recover provenance from other parents, without counting
  shared source occurrences twice or attributing inherited moves to the merger.

- Blank and whitespace-only lines now carry zero ownership and labor. Blank-only
  additions, removals and relocations are formatting and preserve meaningful owners.
  Mixed edits also align only meaningful lines, then restore physical blank positions.
  This corrects the unreleased algorithm version 1 implementation.
- Rename attribution no longer depends on staged/worktree attributes, configured
  attribute files, diff drivers or text conversion.

### Performance

- Share token spellings and owners, borrow intern keys with deterministic hashing,
  dispatch operators by first byte, and bypass Myers for disjoint vocabularies.
  On the 2,118-commit fd benchmark, token replay stays below three times line replay.
- Reuse one isolated raw-object diff cache per analysis, clearing transient blobs
  between commits.
- Use exact bit-parallel LCS for long lines with bounded alphabets, cache character
  frequencies, prune provably inferior pairs, and index physical move prefixes.
  Scores and deterministic tie-breaking remain unchanged.
