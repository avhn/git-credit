# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Paired light/dark activity and ownership-history SVG charts, with deterministic
  calendar axes, weekly gaps, monthly aggregation for long activity spans, shared
  contributor colors and privacy-filtered top-N legends.
- Weekly activity sparkline below `--timeline` TTY tables, showing the last 52
  calendar weeks, period and peak labor, with Unicode and ASCII levels.
- `badge --style activity` and `history`, included in `all`, automatic timeline
  analysis, optional timeline report import and generated-asset publication support.
- Opt-in `analyze --timeline` and config `timeline = true`: UTC weekly activity by
  author time and full-graph ownership sampled along HEAD's first-parent ancestry.
- Exact integer historical totals with report-local author references and final
  alias/opt-out privacy mapping across every historical point.

### Changed

- Cache format 3 validates ownership summaries and ordered identity allocation;
  missing samples or reordered rounding identities trigger complete reconstruction.

## [0.1.1] - 2026-11-09

### Added

- Four-platform Homebrew formula publication to `avhn/homebrew-tap`, preserving the
  existing fortix formula and README entries, with macOS and Linux install checks.
- Deterministic amd64 and arm64 Debian packages, release checksums, and a signed
  stable/main APT repository on GitHub Pages with retained package versions.
- Manual Pages rebuild workflow, packaging guard tests, installation instructions
  and signing-key rotation documentation.

### Changed

- Linux release binaries now use static musl instead of GNU/glibc targets for
  Debian 11+ and Ubuntu 20.04+ portability. The GitHub Action prefers musl assets
  while retaining manifest-based compatibility with v0.1.0 GNU archives.

## [0.1.0] - 2026-11-09

### Changed

- Algorithm version 4 adds bounded fuzzy same-commit token moves, preserving matched
  token ownership and charging only unmatched editor tokens. Line rules are unchanged.

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

- Default incremental replay cache inside the Git directory, content-addressed file
  states, recent first-parent snapshots, atomic checksummed generations, safe cold
  fallback, and byte-equivalence fixtures for merges, rewrites and invalid caches.
- `--cache-dir`, `--cache-depth`, `--no-cache`, config `cache = false`, and a pinned
  GitHub Action cache input used by the badge workflow.
- Release cold and incremental benchmarks on fd, ripgrep and tokio in
  `docs/PERFORMANCE.md`.

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

[Unreleased]: https://github.com/avhn/git-credit/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/avhn/git-credit/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/avhn/git-credit/releases/tag/v0.1.0
