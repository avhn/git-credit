#!/usr/bin/env bash
# Package a static musl binary without executing it or adding runtime dependencies.
# Usage: package-deb.sh VERSION ARCH BINARY OUTDIR
# ARCH is amd64 or arm64; SOURCE_DATE_EPOCH fixes all package timestamps (default 0).
# Requires Linux dpkg-deb and GNU touch/date; output packages are never overwritten.
set -euo pipefail

# die explains invalid packaging inputs before publishing any package bytes.
die() {
    printf '%s\n' "${*}" >&2
    exit 1
}

[[ $# -eq 4 ]] || die 'usage: package-deb.sh VERSION ARCH BINARY OUTDIR'
VERSION="${1#v}"
ARCH="${2}"
BINARY="${3}"
OUTPUT="${4}"
[[ "${VERSION}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die 'invalid stable release version'
[[ "${ARCH}" == amd64 || "${ARCH}" == arm64 ]] || die 'unsupported Debian architecture'
[[ -f "${BINARY}" && ! -L "${BINARY}" && -s "${BINARY}" ]] || die 'invalid binary'
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-0}"
[[ "${SOURCE_DATE_EPOCH}" =~ ^[0-9]+$ ]] || die 'invalid SOURCE_DATE_EPOCH'
[[ ! -L "${OUTPUT}" ]] || die 'output is a symlink'
mkdir -p "${OUTPUT}"
PACKAGE="${OUTPUT}/git-credit_${VERSION}-1_${ARCH}.deb"
[[ ! -e "${PACKAGE}" && ! -L "${PACKAGE}" ]] || die 'output already exists'
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
STAGING="$(mktemp -d "${TMPDIR:-/tmp}/git-credit-deb.XXXXXX")"
trap 'rm -rf "${STAGING}"' EXIT
DOC="${STAGING}/root/usr/share/doc/git-credit"
mkdir -p "${STAGING}/root/DEBIAN" "${STAGING}/root/usr/bin" "${DOC}"
install -m 0755 "${BINARY}" "${STAGING}/root/usr/bin/git-credit"
install -m 0644 "${ROOT}/README.md" "${DOC}/README.md"
mkdir -p "${STAGING}/root/usr/share/lintian/overrides"
install -m 0644 "${ROOT}/packaging/debian/lintian-overrides" "${STAGING}/root/usr/share/lintian/overrides/git-credit"
cat >"${STAGING}/root/DEBIAN/control" <<EOF
Package: git-credit
Version: ${VERSION}-1
Architecture: ${ARCH}
Maintainer: Mert Dede <doganmertdede@gmail.com>
Section: vcs
Priority: optional
Recommends: git
Homepage: https://github.com/avhn/git-credit
Description: fair git authorship stats
 Reports surviving meaningful code ownership and historical labor without
 letting formatting or trivial edits transfer credit to the last editor.
EOF
{
    cat <<'EOF'
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: git-credit
Source: https://github.com/avhn/git-credit

Files: *
Copyright: 2026 Mert Dede
License: MIT or Apache-2.0

License: MIT
EOF
    sed -e 's/^$/./' -e 's/^/ /' "${ROOT}/LICENSE-MIT"
    cat <<'EOF'

License: Apache-2.0
 On Debian systems, the complete text of the Apache License, Version 2.0
 can be found in /usr/share/common-licenses/Apache-2.0.
EOF
} >"${DOC}/copyright"
DATE="$(LC_ALL=C date -u -d "@${SOURCE_DATE_EPOCH}" '+%a, %d %b %Y %H:%M:%S +0000')"
cat >"${STAGING}/changelog" <<EOF
git-credit (${VERSION}-1) stable; urgency=medium

  * Package upstream ${VERSION} with a portable static musl binary.

 -- Mert Dede <doganmertdede@gmail.com>  ${DATE}
EOF
gzip -n -9 -c "${STAGING}/changelog" >"${DOC}/changelog.Debian.gz"
gzip -n -9 -c "${ROOT}/CHANGELOG.md" >"${DOC}/changelog.gz"
# Normalize permissions and timestamps, including directories and compressed docs.
find "${STAGING}/root" -type d -exec chmod 0755 {} +
find "${STAGING}/root" -type f ! -path '*/usr/bin/git-credit' -exec chmod 0644 {} +
find "${STAGING}/root" -exec touch -d "@${SOURCE_DATE_EPOCH}" {} +
dpkg-deb --root-owner-group -Zxz --build "${STAGING}/root" "${PACKAGE}" >/dev/null
