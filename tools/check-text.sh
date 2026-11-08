#!/usr/bin/env bash
# check-text.sh: repository text policy for every tracked file.
#   - no en dash (U+2013) or em dash (U+2014): use a hyphen, comma, colon or parentheses
#   - no CRLF line endings in tracked text (the index must hold LF)
# Exits non-zero and names each offending file. Byte patterns under LC_ALL=C
# keep the dash check portable across GNU and BSD grep.
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
cd "$root"

en=$'\xe2\x80\x93'
em=$'\xe2\x80\x94'
status=0

while IFS= read -r -d '' file; do
    [[ -f "$file" ]] || continue
    case "$file" in *.png | *.jpg | *.gif | *.ico | *.woff*) continue ;; esac
    if LC_ALL=C grep -q -e "$en" -e "$em" -- "$file"; then
        LC_ALL=C grep -n -e "$en" -e "$em" -- "$file" | cut -d: -f1 |
            sed "s|^|banned dash in $file line |" >&2
        status=1
    fi
done < <(git ls-files -z)

# `i/crlf` reports text whose committed form contains CRLF.
crlf="$(git ls-files --eol | awk '$1 == "i/crlf" || $1 == "i/mixed" { print $NF }')"
if [[ -n "$crlf" ]]; then
    printf 'CRLF line endings in index: %s\n' $crlf >&2
    status=1
fi

exit "$status"
