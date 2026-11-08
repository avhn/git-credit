#!/usr/bin/env bash
# check-commit-msg.sh: validate commit messages against the repository convention.
#
#   check-commit-msg.sh --file PATH    validate one message file (commit-msg hook)
#   check-commit-msg.sh --range A..B   validate every non-merge commit in a range (CI)
#
# Convention: `type(scope): summary` (scope optional), lowercase summary start,
# no trailing period, header at most 72 characters, body lines at most 100.
set -euo pipefail

types='feat|fix|docs|style|refactor|perf|test|build|ci|chore|revert'
header_re="^($types)(\([a-z0-9][a-z0-9-]*\))?!?: [a-z0-9\`].*[^.]\$"

# check_message LABEL: read a message on stdin, report violations, return 1 on any.
check_message() {
    local label="$1" header line n=0 bad=0
    IFS= read -r header || true
    if [[ ! "$header" =~ $header_re ]]; then
        echo "$label: header must match 'type(scope): lowercase summary' without a trailing period: $header" >&2
        bad=1
    fi
    if (( ${#header} > 72 )); then
        echo "$label: header longer than 72 characters" >&2
        bad=1
    fi
    while IFS= read -r line || [[ -n "$line" ]]; do
        n=$((n + 1))
        [[ "$line" == \#* ]] && continue
        if (( ${#line} > 100 )); then
            echo "$label: body line $((n + 1)) longer than 100 characters" >&2
            bad=1
        fi
    done
    return "$bad"
}

case "${1:-}" in
--file)
    check_message "commit message" < "$2"
    ;;
--range)
    status=0
    while IFS= read -r sha; do
        git log -1 --format=%B "$sha" | check_message "commit ${sha:0:12}" || status=1
    done < <(git rev-list --no-merges "$2")
    exit "$status"
    ;;
*)
    echo "usage: $0 --file PATH | --range A..B" >&2
    exit 2
    ;;
esac
