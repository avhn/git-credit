#!/usr/bin/env bash
# Tests for tools/check-commit-msg.sh: each case feeds a message file and
# asserts accept (0) or reject (1).
set -euo pipefail

checker="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/check-commit-msg.sh"
tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT
failures=0

# expect WANT MESSAGE: WANT is "accept" or "reject".
expect() {
    local want="$1" got
    printf '%b' "$2" > "$tmp"
    if "$checker" --file "$tmp" 2>/dev/null; then got=accept; else got=reject; fi
    if [[ "$got" != "$want" ]]; then
        printf 'FAIL: expected %s for %q\n' "$want" "$2" >&2
        failures=$((failures + 1))
    fi
}

expect accept 'feat(core): add line similarity\n'
expect accept 'fix: handle empty repositories\n\nBody text.\n'
expect accept 'ci(static)!: require pedantic clippy\n'
expect accept 'docs: explain `--json` output\n'
expect reject 'Add line similarity\n'
expect reject 'feat(core): Add line similarity\n'
expect reject 'feat(core): add line similarity.\n'
expect reject 'feature(core): add line similarity\n'
expect reject 'feat(Core): add line similarity\n'
expect reject "feat: $(printf 'x%.0s' {1..80})\n"
expect reject "feat: ok\n\n$(printf 'y%.0s' {1..101})\n"
expect accept "feat: ok\n\n# $(printf 'z%.0s' {1..120})\n"

if (( failures > 0 )); then
    echo "$failures commit message case(s) failed" >&2
    exit 1
fi
echo "commit message checker: all cases passed"
