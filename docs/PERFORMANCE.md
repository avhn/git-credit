# Replay performance

Release-mode observations on 2026-11-09, Windows 11 Pro x64 (10.0.26300),
Intel Core i5-13400F, Rust 1.99.1, algorithm version 4. Builds use the workspace
release profile. Each measurement runs one CLI process without parallel analysis.
These are local wall-clock observations, not portable performance guarantees.

## Method

Repositories are full public clones under `/tmp/gc-bench/` in Git Bash. No code,
filters or hooks from those repositories is executed. Default exclusions, full
history, default policy and directory depth are used. Cold means `--no-cache`,
not an empty operating-system filesystem cache. JSON stdout and verbose stderr
are redirected to files. Each table entry is one run, not a statistical median.

Wall time uses Python `time.perf_counter` around the process, with a 20 ms polling
interval. Peak working set uses Windows `GetProcessMemoryInfo` and its
`PeakWorkingSetSize` field sampled during execution. The CLI does not spawn replay
children. The final interval after the last sample may miss a short-lived peak;
these figures are measured process working sets, not allocation totals or system
memory consumption. `/usr/bin/time` is not required.

| Repository | HEAD | Reachable commits |
| --- | --- | ---: |
| [fd](https://github.com/sharkdp/fd) | `39886c0cb34453cda21163df0c0c76fc9c011a21` | 2,118 |
| [ripgrep](https://github.com/BurntSushi/ripgrep) | `91f5d22ef48bde459d90a8c7dbeab416e97d0f2b` | 2,303 |
| [tokio](https://github.com/tokio-rs/tokio) | `99dbe548b7046915cd56e570e31604e1f8fa83cd` | 4,873 |

## Cold replay

| Repository | Token time (s) | Token peak (MiB) | Line time (s) | Line peak (MiB) |
| --- | ---: | ---: | ---: | ---: |
| fd | 7.783 | 50.0 | 2.630 | 24.1 |
| ripgrep | 14.887 | 96.6 | 2.850 | 37.2 |
| tokio | 59.246 | 368.5 | 14.878 | 126.4 |

## Incremental replay

The fd cache is seeded at first-parent `HEAD~20`, commit
`08a3a48a5a26302e463a2ccedc61da2598536664`, using a temporary detached worktree.
That commit has 2,066 reachable commits. Moving to HEAD introduces 52 reachable
commits, not just 20, because the full walk includes merged branch commits.
The default 64 ancestor snapshots are retained. The original clone's checkout
is never changed; the temporary worktree is removed afterward.

| fd token run | Scored commits | Time (s) | Peak (MiB) |
| --- | ---: | ---: | ---: |
| Cache seed at HEAD~20 | 2,066 | 7.830 | 62.1 |
| Cold HEAD | 2,118 | 7.783 | 50.0 |
| Incremental HEAD | 52 | 0.757 | 116.4 |

Incremental replay is 10.3x faster than cold HEAD in this run. The resulting pack
is 4,122,938 bytes with 65 snapshots, below the 256 MiB cap. Verbose diagnostics
confirm that 2,066 commits were reused and only 52 were replayed.

The incremental stdout is compared byte-for-byte with cold token-mode stdout.
No timestamps or cache metrics are included in JSON. Resume still walks commit
metadata, resolves identities in cold-walk order, validates digests and restores
file provenance; only new commits undergo tree comparison and scoring.

## Reproduce

Build the CLI in this repository, not inside downloaded benchmark repositories.
Use the recorded commits above when comparing future implementations.

```bash
cargo build --release --locked
binary="$(pwd)/target/release/git-credit.exe"
# Use target/release/git-credit on Unix.
for repo in fd ripgrep tokio; do
  for mode in token line; do
    time "$binary" analyze "/tmp/gc-bench/$repo" --granularity "$mode" \
      --json --no-cache > "/tmp/$repo-$mode-cold.json"
  done
done

git -C /tmp/gc-bench/fd -c core.hooksPath=/dev/null worktree add \
  --detach /tmp/gc-bench/fd-base HEAD~20
cache="$(git -C /tmp/gc-bench/fd rev-parse --absolute-git-dir)/git-credit/benchmark-cache"
"$binary" analyze /tmp/gc-bench/fd-base --json --cache-dir "$cache" > /tmp/fd-seed.json
time "$binary" analyze /tmp/gc-bench/fd --json -v --cache-dir "$cache" > /tmp/fd-incremental.json
cmp /tmp/fd-token-cold.json /tmp/fd-incremental.json
git -C /tmp/gc-bench/fd worktree remove /tmp/gc-bench/fd-base
```

Cache size is capped at 256 MiB. Retaining recent snapshots increases memory
relative to cold replay, in exchange for supporting recent-fork merges. Deep
forks, rewritten history, changed scoring policy or changed identity membership
can require cold fallback; `-v` explains the reason. Newly interleaved branches
can renumber author IDs without fallback because cached IDs are remapped through
stable identity-group keys before validating per-commit shares. Timeline summaries
are stricter: numeric identity reordering requires reconstruction because already
rounded ownership totals cannot safely be remapped.

Fuzzy token moves use at most 32 candidates per inserted block, each bounded to
4096 tokens. Large blocks retain exact-move fallback. These conservative limits
and the identity/cache design are documented in [PLAN.md](PLAN.md#phase-3-scale-done).

## Optional timeline

Measured on the same host and fd HEAD listed above, with release binaries on
2026-11-09. Each entry is one serial invocation, using Bash wall-clock `time`,
JSON redirected to disk, no execution of downloaded repository code or hooks.
Cold is `--no-cache`. Separate caches were seeded for each mode; the original
checkout was not changed. These are observations, not statistical guarantees.

| fd run | Without timeline (s) | Activity plus ownership (s) | Overhead |
| --- | ---: | ---: | ---: |
| Cold HEAD, 2,118 commits | 8.023 | 8.374 | 4.4% |
| Incremental from HEAD~1, 2 new commits | 0.579 | 0.594 | 2.6% |
| HEAD~20 boundary, 52 new commits | 0.779 | 8.285 (cold fallback) | 963.5% |

The HEAD~1 seed is `e5849a0` (2,116 reachable commits). Both modes report a real
resume at that boundary. Fully cached and incremental timeline JSON compare
byte-for-byte with cold JSON. The resulting timeline pack is 4,185,865 bytes,
including 65 ordinary replay snapshots, below the 256 MiB cap.

HEAD~20 introduces an earlier occurrence of an existing identity through a merged
branch. The timeline allocation fingerprint correctly rejects those rounded
summaries and reconstructs all 2,118 commits. This is required correctness fallback,
not sampling overhead, and cannot meet the incremental overhead target for that
boundary. Its output still matches cold exactly.

Both-series overhead at the resumable boundary and on cold replay is below 15%
and 35%. There is no public activity-only switch, so an independent activity-only
process measurement was not taken. The measurements above include activity,
ownership sampling, timestamp reads, privacy remapping and JSON serialization;
no hotspot optimization was needed for these sampled runs.
