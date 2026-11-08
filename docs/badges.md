# Contribution badges

`git-credit` produces fair git authorship stats as SVG badges and summary cards.
Ownership describes surviving meaningful code. Labor includes meaningful historical
work, even when it was later deleted. Neither metric measures a person's value.

## Render locally

```console
git-credit badge . --out-dir .git-credit
git-credit badge ../project --out-dir .git-credit --style card --theme dark
git-credit badge --from credit.json --out-dir .git-credit --top 8 --width 800
```

The command is offline. Without `--from`, it uses the same full-history analysis,
root `.git-credit.toml`, exclusions, identity grouping, mailmap and opt-out policy
as `analyze` with its default CLI options. Put policy overrides in configuration,
or run `analyze` with your overrides and import that JSON. Shallow repositories
fail with a `git fetch --unshallow` hint; no fetching occurs during analysis.

| CLI option | Default | Meaning |
| --- | --- | --- |
| `PATH` | `.` | Repository to analyze; ignored with `--from` |
| `--out-dir DIR` | Required | Directory for generated assets |
| `--style` | `all` | One of `bar`, `card`, `bus-factor`, `authors`, `all` |
| `--top N` | `5` | Named contributors, from 1 to 1000 |
| `--theme` | `light` | `light` or `dark`, applied to the summary card |
| `--width PX` | `600` | Bar and card width, from 200 to 4096 pixels |
| `--from REPORT_JSON` | None | Import a privacy-filtered [schema 1 report](json.md), without opening a repository |
| `--cache-dir DIR` | Inside Git directory | Override replay cache location |
| `--cache-depth N` | `64` | Retain N first-parent ancestors plus HEAD |
| `--no-cache` | Off | Disable replay cache reads and writes |
| `-v` | Off | List written paths and analysis warnings on stderr |

Successful runs leave stdout empty. Invalid flags exit 2; report, analysis and
filesystem errors exit 1. Identical input and options produce identical asset
bytes. Card and legend labels fit conservative Verdana 11px metrics, with full
names in SVG titles. Non-ASCII names are supported and all SVG text is escaped.
The bar uses rank-based colors plus text labels. Legend tenths use largest-remainder
rounding, so the named contributors plus others sum to 100.0%. A report with no live
ownership produces a valid `no data` ownership badge, not a fictitious bus factor.

### Output files

| File | When written |
| --- | --- |
| `credit-bar.svg` | `bar` or `all`: top N stacked ownership shares plus others and a legend |
| `credit-card.svg` | `card` or `all`: top N ownership percentages and labor, commit count and short HEAD |
| `bus-factor.svg` | `bus-factor` or `all`: minimum ranked contributors strictly exceeding 50% ownership |
| `authors/author-<slug>.svg` | `authors` or `all`: each top contributor's owned percentage |
| `bus-factor.json` | Always: shields.io endpoint with red (1), yellow (2), green (3+) or no-data gray |
| `top-contributor.json` | Always: shields.io endpoint with the leading name and whole ownership percent |
| `credit.json` | Always: the analyze JSON report, including policy and directory data |

Names become bounded ASCII filename components prefixed with `author-`, avoiding
path traversal and Windows device names. Deterministic numeric suffixes resolve
collisions, including duplicate names and names without ASCII letters. Old managed
SVGs are removed when styles or identities change; unrelated files are preserved.
Use a dedicated output directory. Symlinked output paths are refused. Do not modify
the directory concurrently with rendering.

The bus factor is a concentration statistic, not a prediction of team resilience:
exactly 50% does not exceed the threshold. Two equal owners yield 2, four equal
owners yield 3. Historical labor does not change this ownership-based calculation.

## Add badges to a README

Publish generated files to a branch and embed their raw URLs:

```markdown
![Contributions](https://raw.githubusercontent.com/OWNER/REPO/badges/credit-bar.svg)
![Contribution summary](https://raw.githubusercontent.com/OWNER/REPO/badges/credit-card.svg)
![Bus factor](https://raw.githubusercontent.com/OWNER/REPO/badges/bus-factor.svg)
```

Replace `OWNER/REPO`; URLs become live after the first successful publication.
GitHub's image proxy may cache earlier images. Link a particular author using its
actual generated filename rather than assuming its slug.

For shields.io endpoints, serve the JSON over HTTPS and use a URL-encoded endpoint:

```text
https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2FOWNER%2FREPO%2Fbadges%2Fbus-factor.json
```

## GitHub Action

The reusable composite action downloads a release for Linux x86_64/ARM64, macOS
x86_64/ARM64 or Windows x86_64, verifies its SHA-256 against that release's
`SHA256SUMS`, and extracts only the regular binary. Checksums detect corruption;
they do not provide independent authenticity if a release account is compromised.
Pin the action to a reviewed full commit SHA and `version` to a released tag for
repeatability. Replace `FULL_COMMIT_SHA` in the example with the reviewed release's
40-character commit SHA after the first release is available.

```yaml
name: Contribution badges
on:
  push:
    branches: [main]
permissions:
  contents: read
concurrency:
  group: badges
  cancel-in-progress: false
jobs:
  badges:
    runs-on: ubuntu-latest
    timeout-minutes: 30
    permissions:
      contents: write
    steps:
      - name: Check out source
        uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          fetch-depth: 0
          persist-credentials: false
      - name: Publish contribution badges
        uses: avhn/git-credit@FULL_COMMIT_SHA # Replace with the reviewed release commit.
        with:
          source-branch: main
          version: latest
```

### Action inputs

| Input | Default | Meaning |
| --- | --- | --- |
| `cache` | `true` | Restore/save incremental replay state with actions/cache; false passes --no-cache |
| `version` | `latest` | Download the latest release or a tag such as `v0.1.0` |
| `binary` | Empty | Use an existing executable path, skipping download/checksum verification |
| `out-dir` | `.git-credit` | Generated directory, relative to the checkout unless absolute |
| `styles` | `all` | One CLI style selection, not a comma-separated list |
| `top` | `5` | Number of named contributors |
| `publish-branch` | `badges` | Branch to replace; empty skips publication |
| `token` | `${{ github.token }}` | Token with `contents: write` when publishing |
| `source-branch` | Empty | Skip publication unless its current remote HEAD still equals the triggering SHA |

Cache keys include repository, runner OS, algorithm version and triggering SHA,
with a same-repository/OS/algorithm restore prefix. The CLI validates restored
policy, identities and ancestry before use. Cache files contain historical
provenance, not privacy-filtered assets; GitHub cache access and retention rules
apply. Disable the cache input if that retention is inappropriate.

The caller must check out with `fetch-depth: 0`. The action fails clearly on a
shallow checkout, rather than reporting inflated root-author credit. It does not
need `contents: write` when `publish-branch` is empty. The action cannot declare
caller permissions; grant only what the selected mode needs. No workflow runs on
pull requests and no pull-request code should receive a publishing token.

Publication stages only the recognized generated assets into a separate temporary
repository. Its commit is a fresh orphan with no source files or prior history,
created by `github-actions[bot]` as `chore(badges): update for <short sha>`.
The source/default branch is refused as a publication target. Existing target refs
use `--force-with-lease` against the fetched object ID; absent refs use a
creation-only empty lease. A concurrent change causes failure rather than being
overwritten. Protected badge branches may need a narrowly scoped rules exception.
A mismatched checked-out/triggering SHA is rejected. Use `source-branch: main` and
a non-cancelling concurrency group to prevent stale runs from replacing newer
results; this repository's [dogfood workflow](../.github/workflows/badges.yml)
builds from source and uses that guard immediately before pushing.

## Privacy and retention

- Badge analysis applies opt-out and exclusions before rendering. Opted-out groups
  appear only as `anonymous`, and their ownership remains in the denominator.
- Import mode trusts the supplied report. It does not reapply today's opt-out
  configuration. Regenerate reports after privacy-policy changes before importing.
- `credit.json` contains public non-opted-out canonical emails and directory data.
  Publishing makes that full report public, not just the names visible in SVGs.
  Use no-publish mode and selectively host SVGs if that is inappropriate.
- Re-rendering removes stale managed author SVGs. Orphan publishing removes previous
  assets from the new branch tip, but cannot erase Git objects, clones, CDN caches,
  screenshots or previously downloaded reports. An opt-out is not guaranteed
  retroactive deletion from external copies.
- Never place credentials or unrelated private files in the output directory. The
  publisher uses a strict filename allowlist and refuses symlinks, but intentionally
  publishes the complete generated `credit.json` report.

## Releases

Push `v<workspace version>` to build five native platform archives containing the
binary, README and both licenses. Linux ARM64 uses `ubuntu-24.04-arm`; macOS builds
both Apple targets with the Rust target toolchains. Windows uses zip, other targets
use tar.gz. The release workflow checks version equality, policy, static checks,
tests and all builds before `gh release create` uploads archives and `SHA256SUMS`.
Only the final publication job has `contents: write`. Tag-triggered releases are a
deliberate policy choice for this open source CLI. Until the first tagged release
exists, use a source build with the action's `binary` input.
