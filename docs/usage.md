# Usage reference

Everything the README leaves out: every flag, configuration, identity handling,
the cache, and what is and is not scored. Run `git-credit analyze --help` for the
same flag list in your terminal. (`git credit --help` asks Git for a man page, which
is not shipped yet; use `git credit -h` or `git-credit --help`.)

## Commands

| Command                          | What it does                                                     |
| -------------------------------- | ---------------------------------------------------------------- |
| `git credit analyze [PATH]`      | Ownership and labor per author, as a table or `--json`           |
| `git credit badge [PATH]`        | Render SVG badges, a summary card and shields.io endpoint JSON   |

`PATH` defaults to the current directory and may be a working tree or a bare
repository. Only committed content at `HEAD` is analyzed; uncommitted changes are
ignored.

## `analyze` flags

| Flag                         | Default        | Effect                                                                  |
| ---------------------------- | -------------- | ----------------------------------------------------------------------- |
| `--json`                     | off            | Versioned JSON report instead of a table ([contract](json.md))          |
| `--depth N`                  | `1`            | Directory breakdown depth for JSON and `--tree`; `0` disables it                      |
| `--exclude GLOB`             | none           | Skip matching paths; repeat the flag; replaces config exclusions        |
| `--by-email`                 | off            | Do not merge identities that share a display name                       |
| `--granularity line\|token`  | `token`        | Attribution unit; reports always use line-equivalent units              |
| `--keep 0.0..1.0`            | `0.70`         | Line mode only: similarity needed for a line to keep its owner          |
| `--no-bulk-discount`         | off            | Treat bulk renames as ordinary edits                                    |
| `--first-parent`             | off            | Walk only the first-parent chain (merge content goes to the merger)     |
| `--no-cache`                 | off            | Neither read nor write the incremental cache                            |
| `--cache-dir DIR`            | in `.git`      | Cache location                                                          |
| `--cache-depth N`            | `64`           | First-parent ancestor snapshots kept in addition to `HEAD`              |
| `-q`, `--quiet`              | off            | Hide warnings; results and errors still print                           |
| `-v`, `-vv`                  | off            | Diagnostics on stderr (cache decisions, skipped files)                  |
| `--no-color`                 | off            | Disable ANSI color; nonempty `NO_COLOR` also disables it                 |
| `--color auto\|always\|never` | `auto`         | Color on terminals, explicitly force ANSI, or never use ANSI             |
| `--ascii`                    | off            | ASCII ownership bars and tree branches instead of Unicode               |
| `--bars`                     | TTY only       | Include ownership bars even in redirected output                        |
| `--no-bars`                  | off            | Hide ownership bars, including on terminals                             |
| `--tree`                     | off            | Indented directory ownership with the top three owners; conflicts with `--json` |

Exit codes: `0` success, `1` analysis or configuration error, `2` invalid usage.
Results go to stdout and diagnostics to stderr, so `--json` output pipes cleanly.

## Reading the table

```console
$ git credit analyze
AUTHOR  OWNERSHIP %  OWNED LINES  LABOR
Alice         79.5%         23.9  17.0
Dave          20.0%          6.0  6.0
Carol          0.5%          0.1  0.1
HEAD e5c6d840fbfd | 4 commits | full | token | algorithm 4
```

- **OWNERSHIP %** and **OWNED LINES**: share of the meaningful lines at `HEAD`.
  A line edited by two people is split between them by token weight, so owned
  lines can be fractional.
- **LABOR**: meaningful work across all history, in line-equivalents. Writing a new
  line earns 1.0; changing one of ten tokens earns 0.1; deleting earns nothing;
  formatting earns nothing. Labor never goes down because someone else edits later.
- **Footer**: analyzed commit, number of commits, walk mode (`full` or
  `first-parent`), granularity and algorithm version. Reports from different
  algorithm versions are not comparable.

### Terminal presentation

Terminals get a bold header, categorical author colors, inline ownership bars and
a dim footer. Warnings are yellow on stderr. Every author still has an explicit
percentage. `--color always` deliberately enables ANSI in redirected output;
`--no-color` and **nonempty** `NO_COLOR` take precedence, even over `always`.
An empty `NO_COLOR` does not disable color. JSON never contains presentation ANSI.

Bars default to 20 cells and shrink with the available `COLUMNS` width (bounded
from 40 to 512 columns; invalid or absent values use 80). The locale hint uses the
first nonempty `LC_ALL`, `LC_CTYPE` or `LANG`. Non-UTF-8 locales, including `C` and
`POSIX`, use `#` and `.`; `--ascii` forces that fallback. With no locale hint,
Unicode is used. Names, paths and diagnostics have terminal controls and bidi
formatting marks neutralized. No terminal settings change attribution.

```console
$ git credit analyze --tree --depth 2 --ascii --no-color
DIRECTORY OWNERSHIP
+- src
  +- parser  Alice 75.0%, Dave 25.0%
```

`--tree` is opt-in and reports the existing directory snapshot at `--depth`, not a
second analysis. Intermediate directories are structural nodes, not duplicated
ownership totals. `--depth 0 --tree` explicitly reports that directory data is
unavailable. Plain redirected `analyze` without these overrides is byte-identical
to the original table; `--json` retains its existing serialization.

## How credit is decided

- **Tokens, not lines.** Each file is split into language-agnostic tokens
  (identifiers, numbers, strings, comment words, operators). Unchanged tokens keep
  their owner; inserted and replaced tokens go to the editor.
- **Formatting is free.** Re-indenting, rewrapping or joining lines and adding a
  trailing comma produce the same token stream, so they earn nothing and take
  nothing. Indentation still counts in Python, YAML, Makefiles and other
  indentation-sensitive files. String contents always count.
- **Blank lines** carry no weight. Punctuation weighs a quarter of a word.
- **Moves keep their author.** A block of three or more meaningful lines moved
  within a commit keeps its owners, even if up to 20% of it was edited on the way;
  only the edited tokens go to the mover.
- **Renames keep their author.** File renames are followed.
- **Bulk renames keep their author.** When one identifier replacement (for example
  `userId` to `accountId`) repeats at least five times and dominates a commit, the
  renamed tokens keep their owners and the editor earns one line of labor per
  rename per file. Literal, keyword and operator changes never count as renames.
- **Merges credit the branch author.** Every commit is visited once. Content a
  merge takes from a parent keeps that parent's attribution; only genuinely new
  conflict-resolution content is credited to the person who merged.

### Line granularity

`--granularity line` uses whole lines instead of tokens. A changed line keeps its
owner when it is at least `--keep` similar (default 70%); the editor earns the
changed fraction as labor. It is faster (about 3x) but cannot see through
rewrapping.

## Identities and privacy

- The `.mailmap` committed at `HEAD` applies to all history (`mailmap.file` and
  `mailmap.blob` configuration are honored; uncommitted `.mailmap` edits are not).
- `Co-authored-by:` trailers split a commit's credit equally.
- By default, identities with the same display name (case-insensitive) are merged,
  and the most used email represents them. Use `--by-email` or `group_by = "email"`
  when different people share a name.
- `opt_out` emails are combined into one anonymous row. Percentages stay correct,
  and none of that person's addresses appear in the output.

## What is not scored

Lockfiles (`Cargo.lock`, `package-lock.json`, `pnpm-lock.yaml`, `yarn.lock`,
`poetry.lock`, `go.sum` and others), `vendor/`, `node_modules/`, `dist/`, minified
files and source maps are always excluded. Binary files, symlinks, submodules and
files larger than `max_file_size` (default 1 MiB) are skipped with a warning.

Shallow clones are refused, since missing history would credit everything to
the oldest visible commit. Run `git fetch --unshallow` first. Analysis never
fetches, never runs hooks, and ignores `.gitattributes` diff drivers. Only SHA-1
repositories are supported.

## Configuration file

Optional `.git-credit.toml` at the repository root. Command-line flags override it.

```toml
walk = "full"             # or "first-parent"
granularity = "token"     # or "line"
keep = 0.70               # line granularity only
bulk_discount = true
cache = true
group_by = "name"         # or "email"
exclude = ["generated/**", "fixtures/**"]
opt_out = ["private@example.com"]
max_file_size = 1048576
```

Repeated `--exclude` flags replace the configured `exclude` list; built-in
exclusions always stay active. Thresholds are rounded to three decimals.

## Incremental cache

Analysis stores replay snapshots in `.git/git-credit/cache/`, never in the working
tree. The next run only processes new commits, and its output is byte-identical to
a full run. A change of policy, a history rewrite, or a merge of a branch that
forked more than `--cache-depth` commits ago falls back to a full run
automatically; `-v` says why. Corrupt caches are discarded and rebuilt, and the
cache is kept under 256 MiB. The cache contains historical provenance, so do not publish
it. Measurements: [PERFORMANCE.md](PERFORMANCE.md).

## Badges

`git credit badge --out-dir DIR` writes `credit-bar.svg`, `credit-card.svg`,
`bus-factor.svg`, `authors/*.svg`, shields.io endpoint files and `credit.json`.
`--style bar|card|donut|labor|treemap|bus-factor|authors|all`, `--top N`,
`--depth N` and `--width PX` select the output. Every SVG gets light and `-dark`
variants; legacy `--theme light|dark` is accepted but no longer selects one theme.
`--from credit.json` renders an existing report. The reusable
GitHub Action and its privacy notes are in [badges.md](badges.md).
