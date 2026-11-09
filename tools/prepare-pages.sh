#!/usr/bin/env bash
# Prepare and retain the Pages site, optionally adding and signing a released deb set.
# Usage: prepare-pages.sh rebuild | prepare-pages.sh release DEBS_DIR VERSION
# Requires GH_TOKEN, RUNNER_TEMP, GITHUB_ENV and, for release, the two APT secrets.
# Authentication stays in an HTTP extraheader; no credential reaches a remote URL.
set -euo pipefail
set +x

# die reports a missing publishing prerequisite without printing sensitive input.
die() {
    printf '%s\n' "${*}" >&2
    exit 1
}

# cleanup stops only the isolated signing agent and removes its private directory.
cleanup() {
    if [[ -n "${KEYS:-}" ]]; then
        GNUPGHOME="${KEYS}" gpgconf --kill gpg-agent >/dev/null 2>&1 || true
        rm -rf "${KEYS}"
    fi
}

[[ ($# -eq 1 && "${1}" == rebuild) || ($# -eq 3 && "${1}" == release) ]] || die 'usage: prepare-pages.sh rebuild | release DEBS_DIR VERSION'
[[ -n "${GH_TOKEN:-}" ]] || die 'GH_TOKEN is required'
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly EXPECTED=097DDFE92CE4B0F4E204F434668F8E1D756456FD
KEYS=''
trap cleanup EXIT
if [[ "${1}" == release ]]; then
    [[ -n "${APT_SIGNING_KEY:-}" ]] || die 'APT_SIGNING_KEY is required'
    [[ -n "${APT_SIGNING_PASSPHRASE:-}" ]] || die 'APT_SIGNING_PASSPHRASE is required'
    [[ "${3}" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || die 'invalid stable release version'
    KEYS="$(mktemp -d "${RUNNER_TEMP:?}/git-credit-apt-keys.XXXXXX")"
    export GNUPGHOME="${KEYS}"
    chmod 0700 "${GNUPGHOME}"
    printf '%s\n' "${APT_SIGNING_KEY}" | gpg --batch --import
    unset APT_SIGNING_KEY
    ACTUAL="$(gpg --batch --with-colons --list-secret-keys | awk -F: '$1 == "sec" { count++; primary = 1; next } primary && $1 == "fpr" { print $10; primary = 0 } END { if (count != 1) exit 1 }')" || die 'Imported APT signing key must contain exactly one primary key'
    [[ "${ACTUAL}" == "${EXPECTED}" ]] || die 'Imported APT signing key fingerprint mismatch'
    python3 -I "${ROOT}/tools/verify-apt-repo.py" --key "${ROOT}/packaging/apt/git-credit-archive-keyring.asc" "${EXPECTED}"
fi
export GIT_CONFIG_COUNT=1
export GIT_CONFIG_KEY_0=http.https://github.com/.extraheader
GIT_CONFIG_VALUE_0="AUTHORIZATION: basic $(printf 'x-access-token:%s' "${GH_TOKEN}" | base64 | tr -d '\r\n')"
export GIT_CONFIG_VALUE_0
export GIT_TERMINAL_PROMPT=0
PAGES="$(mktemp -d "${RUNNER_TEMP:?}/git-credit-pages.XXXXXX")"
git -C "${PAGES}" init --quiet
git -C "${PAGES}" remote add origin https://github.com/avhn/git-credit.git
if git -C "${PAGES}" ls-remote --exit-code --heads origin gh-pages >"${PAGES}/ref"; then
    rm "${PAGES}/ref"
    git -C "${PAGES}" fetch --depth 1 origin gh-pages
    git -C "${PAGES}" -c core.hooksPath=/dev/null checkout -b gh-pages FETCH_HEAD
else
    STATUS=$?
    [[ "${STATUS}" -eq 2 ]] || die 'Could not query retained gh-pages branch'
    [[ "${1}" == release ]] || die 'No retained gh-pages branch; publish a release first'
    rm "${PAGES}/ref"
    git -C "${PAGES}" checkout --orphan gh-pages
fi
# Pages upload dereferences links, so reject all retained symlinks before writing.
[[ -z "$(find "${PAGES}" -type l -print -quit)" ]] || die 'Retained Pages repository contains a symlink'
if [[ "${1}" == release ]]; then
    bash "${ROOT}/tools/build-apt-repo.sh" "${2}" "${PAGES}/apt" "${EXPECTED}"
    MESSAGE="git-credit ${3#v} apt repository"
else
    [[ -f "${PAGES}/apt/dists/stable/InRelease" ]] || die 'No signed APT repository; publish a release first'
    python3 -I "${ROOT}/tools/verify-apt-repo.py" "${PAGES}/apt" "${ROOT}/packaging/apt/git-credit-archive-keyring.asc" "${EXPECTED}"
    MESSAGE='git-credit site pages'
fi
bash "${ROOT}/tools/build-pages-index.sh" "${PAGES}" "${EXPECTED}"
touch "${PAGES}/.nojekyll"
git -C "${PAGES}" config user.name 'git-credit-release[bot]'
git -C "${PAGES}" config user.email '41898282+github-actions[bot]@users.noreply.github.com'
git -C "${PAGES}" add index.html apt .nojekyll
if ! git -C "${PAGES}" diff --cached --quiet; then
    git -C "${PAGES}" -c core.hooksPath=/dev/null -c commit.gpgsign=false commit -m "${MESSAGE}"
    # Normal fast-forward push fails if a writer outside the workflow lock intervened.
    git -C "${PAGES}" push origin HEAD:gh-pages
fi
printf 'PAGES_DIRECTORY=%s\n' "${PAGES}" >>"${GITHUB_ENV:?}"
