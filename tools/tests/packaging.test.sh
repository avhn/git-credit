#!/usr/bin/env bash
# Test renderers everywhere and actual Debian packaging/signing when tools are available.
# Usage: packaging.test.sh
# Uses inert binaries and an isolated throwaway signing key, never publication credentials.
set -euo pipefail
# Fixture signing must never inherit publication credentials or transition policy.
unset APT_PREVIOUS_FINGERPRINT APT_SIGNING_KEY APT_SIGNING_PASSPHRASE GH_TOKEN
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
python3 -I -m unittest discover -s "${ROOT}/tools/tests" -p '*_test.py'
for TOOL in dpkg-deb apt-ftparchive gpg gpgv gpgconf; do
    if ! command -v "${TOOL}" >/dev/null; then
        printf 'SKIP: Debian/signing integration tests (%s is unavailable)\n' "${TOOL}"
        exit 0
    fi
done
STAGING="$(mktemp -d "${TMPDIR:-/tmp}/git-credit-packaging.XXXXXX")"
# cleanup terminates only the fixture's GPG agent before deleting its test-owned files.
cleanup() {
    GNUPGHOME="${STAGING}/gnupg" gpgconf --kill gpg-agent >/dev/null 2>&1 || true
    if [[ -d "${STAGING}/wrong-key" ]]; then
        GNUPGHOME="${STAGING}/wrong-key" gpgconf --kill gpg-agent >/dev/null 2>&1 || true
    fi
    rm -rf "${STAGING}"
}
trap cleanup EXIT
# expect_failure requires the intended guard to fail with the expected diagnostic.
expect_failure() {
    local diagnostic="${1}"
    shift
    if "${@}" >"${STAGING}/stdout" 2>"${STAGING}/stderr"; then
        printf 'Expected failure: %s\n' "${*}" >&2
        exit 1
    fi
    grep -Fq "${diagnostic}" "${STAGING}/stderr" || {
        cat "${STAGING}/stderr" >&2
        exit 1
    }
}
printf 'inert binary fixture\n' >"${STAGING}/binary"
# Build both architectures twice and require byte-identical package output.
for VERSION in 0.1.0 0.1.1; do
    for ARCH in amd64 arm64; do
        bash "${ROOT}/tools/package-deb.sh" "${VERSION}" "${ARCH}" "${STAGING}/binary" "${STAGING}/debs-${VERSION}"
        bash "${ROOT}/tools/package-deb.sh" "${VERSION}" "${ARCH}" "${STAGING}/binary" "${STAGING}/repeat-${VERSION}"
        FILE="git-credit_${VERSION}-1_${ARCH}.deb"
        cmp "${STAGING}/debs-${VERSION}/${FILE}" "${STAGING}/repeat-${VERSION}/${FILE}"
        [[ "$(dpkg-deb -f "${STAGING}/debs-${VERSION}/${FILE}" Recommends)" == git ]]
        [[ -z "$(dpkg-deb -f "${STAGING}/debs-${VERSION}/${FILE}" Depends)" ]]
        [[ "$(dpkg-deb -f "${STAGING}/debs-${VERSION}/${FILE}" Section)" == vcs ]]
    done
done
mkdir "${STAGING}/extracted"
dpkg-deb -x "${STAGING}/debs-0.1.1/git-credit_0.1.1-1_amd64.deb" "${STAGING}/extracted"
cmp "${STAGING}/binary" "${STAGING}/extracted/usr/bin/git-credit"
DOC="${STAGING}/extracted/usr/share/doc/git-credit"
grep -Fq 'License: MIT or Apache-2.0' "${DOC}/copyright"
grep -Fq '/usr/share/common-licenses/Apache-2.0' "${DOC}/copyright"
grep -Fxq 'git-credit: statically-linked-binary' "${STAGING}/extracted/usr/share/lintian/overrides/git-credit"
[[ -f "${DOC}/README.md" && -s "${DOC}/changelog.gz" && -s "${DOC}/changelog.Debian.gz" ]]
expect_failure 'output already exists' bash "${ROOT}/tools/package-deb.sh" 0.1.1 amd64 "${STAGING}/binary" "${STAGING}/debs-0.1.1"
ln -s "${STAGING}/binary" "${STAGING}/linked-binary"
expect_failure 'invalid binary' bash "${ROOT}/tools/package-deb.sh" 0.1.1 amd64 "${STAGING}/linked-binary" "${STAGING}/invalid"
export GNUPGHOME="${STAGING}/gnupg"
mkdir -m 0700 "${GNUPGHOME}"
gpg --batch --pinentry-mode loopback --passphrase '' --quick-generate-key \
    'git-credit packaging fixture <packaging@example.invalid>' rsa2048 sign 0
FINGERPRINT="$(gpg --batch --with-colons --list-keys | awk -F: '$1 == "pub" { count++; primary = 1; next } primary && $1 == "fpr" { print $10; primary = 0 } END { if (count != 1) exit 1 }')"
PUBLIC_KEY="${STAGING}/public.asc"
gpg --batch --armor --export "${FINGERPRINT}" >"${PUBLIC_KEY}"
for VERSION in 0.1.0 0.1.1 0.1.1; do
    bash "${ROOT}/tools/build-apt-repo.sh" "${STAGING}/debs-${VERSION}" "${STAGING}/apt" "${FINGERPRINT}" "${PUBLIC_KEY}"
done
[[ "$(find "${STAGING}/apt/pool/main/g/git-credit" -name '*.deb' | wc -l)" -eq 4 ]]
for ARCH in amd64 arm64; do
    INDEX="${STAGING}/apt/dists/stable/main/binary-${ARCH}/Packages"
    grep -Fxq 'Version: 0.1.0-1' "${INDEX}"
    grep -Fxq 'Version: 0.1.1-1' "${INDEX}"
    [[ "$(grep -c '^Architecture:' "${INDEX}")" -eq 2 ]]
    [[ "$(grep -c "^Architecture: ${ARCH}$" "${INDEX}")" -eq 2 ]]
    grep -Fxq "Filename: pool/main/g/git-credit/git-credit_0.1.1-1_${ARCH}.deb" "${INDEX}"
    gzip -dc "${INDEX}.gz" >"${STAGING}/uncompressed"
    cmp "${INDEX}" "${STAGING}/uncompressed"
done
for FIELD in 'Origin: git-credit' 'Label: git-credit' 'Suite: stable' 'Codename: stable' 'Architectures: amd64 arm64' 'Components: main'; do
    grep -Fxq "${FIELD}" "${STAGING}/apt/dists/stable/Release"
done
mkdir -m 0700 "${STAGING}/verify-gnupg"
gpg --homedir "${STAGING}/verify-gnupg" --batch --no-default-keyring \
    --keyring "${STAGING}/apt/git-credit-archive-keyring.gpg" --verify "${STAGING}/apt/dists/stable/InRelease"
gpg --homedir "${STAGING}/verify-gnupg" --batch --no-default-keyring \
    --keyring "${STAGING}/apt/git-credit-archive-keyring.gpg" --verify \
    "${STAGING}/apt/dists/stable/Release.gpg" "${STAGING}/apt/dists/stable/Release"
# The old signed manifest must authenticate indexes and every pre-existing pool byte.
cp -R "${STAGING}/apt" "${STAGING}/tampered"
python3 -I - "${STAGING}/tampered/pool/main/g/git-credit/git-credit_0.1.0-1_amd64.deb" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
data = bytearray(path.read_bytes())
data[-1] ^= 1
path.write_bytes(data)
PY
expect_failure 'retained file SHA-256 mismatch' bash "${ROOT}/tools/build-apt-repo.sh" \
    "${STAGING}/debs-0.1.1" "${STAGING}/tampered" "${FINGERPRINT}" "${PUBLIC_KEY}"
cp -R "${STAGING}/apt" "${STAGING}/extra"
cp "${STAGING}/debs-0.1.0/git-credit_0.1.0-1_amd64.deb" \
    "${STAGING}/extra/pool/main/g/git-credit/git-credit_0.0.9-1_amd64.deb"
expect_failure 'unlisted or missing retained Debian package' bash "${ROOT}/tools/build-apt-repo.sh" \
    "${STAGING}/debs-0.1.1" "${STAGING}/extra" "${FINGERPRINT}" "${PUBLIC_KEY}"
cp -R "${STAGING}/apt" "${STAGING}/index-tamper"
printf '\n' >>"${STAGING}/index-tamper/dists/stable/main/binary-amd64/Packages"
expect_failure 'retained file size or type mismatch' bash "${ROOT}/tools/build-apt-repo.sh" \
    "${STAGING}/debs-0.1.1" "${STAGING}/index-tamper" "${FINGERPRINT}" "${PUBLIC_KEY}"
mkdir -p "${STAGING}/unsigned/pool/main/g/git-credit"
cp "${STAGING}/debs-0.1.0/"*.deb "${STAGING}/unsigned/pool/main/g/git-credit/"
expect_failure 'unsigned retained pool must be empty' bash "${ROOT}/tools/build-apt-repo.sh" \
    "${STAGING}/debs-0.1.1" "${STAGING}/unsigned" "${FINGERPRINT}" "${PUBLIC_KEY}"
# A separate fixture key must not authenticate the retained repository by default.
mkdir -m 0700 "${STAGING}/wrong-key"
gpg --homedir "${STAGING}/wrong-key" --batch --pinentry-mode loopback --passphrase '' \
    --quick-generate-key 'other packaging fixture <other@example.invalid>' rsa2048 sign 0
OTHER_FINGERPRINT="$(gpg --homedir "${STAGING}/wrong-key" --batch --with-colons --list-keys | awk -F: '$1 == "pub" { count++; primary = 1; next } primary && $1 == "fpr" { print $10; primary = 0 } END { if (count != 1) exit 1 }')"
cp -R "${STAGING}/apt" "${STAGING}/wrong-signature"
gpg --homedir "${STAGING}/wrong-key" --batch --yes --pinentry-mode loopback --passphrase '' \
    --output "${STAGING}/wrong-signature/dists/stable/InRelease" \
    --clearsign "${STAGING}/wrong-signature/dists/stable/Release"
expect_failure 'retained InRelease signature verification failed' bash "${ROOT}/tools/build-apt-repo.sh" \
    "${STAGING}/debs-0.1.1" "${STAGING}/wrong-signature" "${FINGERPRINT}" "${PUBLIC_KEY}"
# A reviewed previous key allows one transition, without weakening single-key publication.
mkdir "${STAGING}/rotation"
NEW_PUBLIC="${STAGING}/rotation/git-credit-archive-keyring.asc"
gpg --homedir "${STAGING}/wrong-key" --batch --armor --export "${OTHER_FINGERPRINT}" >"${NEW_PUBLIC}"
cp "${PUBLIC_KEY}" "${STAGING}/rotation/git-credit-previous-archive-keyring.asc"
expect_failure 'retained InRelease signature verification failed' python3 -I "${ROOT}/tools/verify-apt-repo.py" \
    "${STAGING}/apt" "${NEW_PUBLIC}" "${OTHER_FINGERPRINT}"
GNUPGHOME="${STAGING}/wrong-key" APT_PREVIOUS_FINGERPRINT="${FINGERPRINT}" \
    bash "${ROOT}/tools/build-apt-repo.sh" "${STAGING}/debs-0.1.1" "${STAGING}/apt" "${OTHER_FINGERPRINT}" "${NEW_PUBLIC}"
python3 -I "${ROOT}/tools/verify-apt-repo.py" "${STAGING}/apt" "${NEW_PUBLIC}" "${OTHER_FINGERPRINT}"
python3 -I "${ROOT}/tools/verify-apt-repo.py" --key "${STAGING}/apt/git-credit-archive-keyring.gpg" "${OTHER_FINGERPRINT}"
cat "${PUBLIC_KEY}" "${NEW_PUBLIC}" >"${STAGING}/multiple.asc"
expect_failure 'exactly one expected primary key' python3 -I "${ROOT}/tools/verify-apt-repo.py" \
    --key "${STAGING}/multiple.asc" "${FINGERPRINT}"
# Restore a repository signed by the initial fixture for the existing failure guards.
cp -R "${STAGING}/extra/dists/stable" "${STAGING}/apt/dists/"
cp "${STAGING}/extra/git-credit-archive-keyring."{gpg,asc} "${STAGING}/apt/"
cp "${STAGING}/apt/dists/stable/InRelease" "${STAGING}/untouched"
expect_failure 'signing key fingerprint mismatch' bash "${ROOT}/tools/build-apt-repo.sh" \
    "${STAGING}/debs-0.1.1" "${STAGING}/apt" 0000000000000000000000000000000000000000
printf 'changed\n' >>"${STAGING}/debs-0.1.1/git-credit_0.1.1-1_amd64.deb"
expect_failure 'existing Debian package differs' bash "${ROOT}/tools/build-apt-repo.sh" \
    "${STAGING}/debs-0.1.1" "${STAGING}/apt" "${FINGERPRINT}" "${PUBLIC_KEY}"
cmp "${STAGING}/untouched" "${STAGING}/apt/dists/stable/InRelease"
ln -s "${STAGING}/binary" "${STAGING}/apt/unsafe-link"
expect_failure 'repository contains a symlink' bash "${ROOT}/tools/build-apt-repo.sh" \
    "${STAGING}/debs-0.1.0" "${STAGING}/apt" "${FINGERPRINT}" "${PUBLIC_KEY}"
expect_failure 'site contains a symlink' bash "${ROOT}/tools/build-pages-index.sh" "${STAGING}/apt" "${FINGERPRINT}"
printf '%s\n' 'PASS: deterministic debs, signed indexes, retained-pool integrity, wrong-signer rejection, single-key keyrings and key rotation'
