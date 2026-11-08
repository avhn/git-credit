# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

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

- Reuse one isolated raw-object diff cache per analysis, clearing transient blobs
  between commits.
- Use exact bit-parallel LCS for long lines with bounded alphabets, cache character
  frequencies, prune provably inferior pairs, and index physical move prefixes.
  Scores and deterministic tie-breaking remain unchanged.
