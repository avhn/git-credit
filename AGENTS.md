@~/.standards/CORE.md
@~/.standards/AGENT.md
@~/.standards/language/rust.md

# git-credit

Rust CLI that reports per-author **ownership** (surviving meaningful code) and
**labor** (meaningful work over all history) for a Git repository, without letting
formatting or trivial edits transfer credit. Public repository: github.com/avhn/git-credit.

Read `docs/PLAN.md` before changing attribution behavior; it holds the algorithm
decisions and the phase plan. Keep it current when a decision changes.

## Commands

- Build: `cargo build --workspace`
- Test: `cargo test --workspace`
- Lint (same flags as CI): `cargo lint`
- Format: `cargo fmt --all`
- Text policy: `tools/check-text.sh`; tooling tests: `tools/tests/check-commit-msg.test.sh`
- Hooks (once per clone): `git config core.hooksPath .githooks`

Local gate before any push: `cargo fmt --all --check && cargo lint && cargo test --workspace && tools/check-text.sh`.

## Structure and boundaries

- `crates/credit-core`: pure engine. No I/O, no clocks, no global state, no serde.
  Every runtime dependency needs a written justification in `docs/PLAN.md`
  (why std is not enough, license, maintenance).
- `crates/credit-git`: the only crate that touches the repository (gix).
- `crates/git-credit`: binary. Config, orchestration, output. `anyhow` is allowed here only.
- Results go to stdout, diagnostics to stderr; `--json` output shape is versioned and
  documented in `docs/json.md` once it exists.
- Reports embed `credit_core::ALGORITHM_VERSION`; bump it whenever the same history
  and policy could produce different credit.

## Tests

- Fixture repositories are built programmatically in temp dirs with fixed identities
  and dates. Disable hooks and commit signing inside fixtures
  (`-c commit.gpgsign=false -c core.hooksPath=/dev/null`).
- Every attribution rule gets a fixture proving it: formatting commit, small edit under
  and over the threshold, move, rename, co-author, mailmap, deletion keeps labor.
- `credit-core` line coverage must stay at or above 80% (enforced in CI).

## Deviations from the org standards (recorded decisions)

- **Commit hooks are Rust-native, not Husky/commitlint/Commitizen.** Owner decision
  2026-11-08: no Node toolchain in a Rust repo. `.githooks/commit-msg` and the CI
  `policy` job run `tools/check-commit-msg.sh` instead; `.githooks/pre-commit` runs
  fmt and the text policy.
- **GitHub-hosted runners, not self-hosted containers.** Public open source project;
  jobs run on `ubuntu-latest`, `windows-latest` and `macos-latest` without a container.
- **Workflow checker not vendored.** The Node-based workflow standard checker is not
  copied in; workflow headers, taxonomy names and SHA pins are maintained by hand.
- **Public README badges.** CI, audit and license badges are shown because this is a
  public open source project.
- **Dependabot is enabled.** Its pull requests are update notices only; changes still
  land on `main` through the normal local flow.
- **Tagged CLI releases.** `release.yml` runs on `v*` tag pushes, rather than only
  branch pushes: tagged releases are the norm for open source CLIs. Tag/workspace
  version equality, policy, static checks, tests and all target builds gate release.
  Only its publication job can write repository contents.
- **Badge publication is generated-only.** `badges.yml` publishes an orphan assets
  branch on main pushes, using an explicit branch lease and a current-main SHA
  check. No source commits are rewritten. The composite action uses the same
  publisher and allows callers to disable publication.
