#!/usr/bin/env bash
# Render the four-platform formula from a release's SHA256SUMS.
# Usage: render-homebrew.sh VERSION CHECKSUMS_FILE OUTDIR
# VERSION is stable, optionally v-prefixed; OUTDIR must not exist.
set -euo pipefail

# die reports invalid input before a partial formula can reach the tap.
die() {
    printf '%s\n' "${*}" >&2
    exit 1
}

# checksum emits exactly one lowercase SHA-256 for the requested asset basename.
checksum() {
    local name="${1}" hash
    hash="$(awk -v name="${name}" '$2 == name { hash = $1; count++; if (NF != 2) bad = 1 } END { if (count != 1 || bad) exit 1; print hash }' "${CHECKSUMS}")" || die "missing or duplicate checksum: ${name}"
    [[ "${hash}" =~ ^[0-9a-f]{64}$ ]] || die "invalid checksum: ${name}"
    printf '%s\n' "${hash}"
}

[[ $# -eq 3 ]] || die 'usage: render-homebrew.sh VERSION CHECKSUMS_FILE OUTDIR'
VERSION="${1#v}"
[[ "${VERSION}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die 'invalid stable release version'
CHECKSUMS="${2}"
[[ -f "${CHECKSUMS}" && ! -L "${CHECKSUMS}" && -s "${CHECKSUMS}" ]] || die 'invalid checksum file'
OUTPUT="${3}"
[[ ! -e "${OUTPUT}" && ! -L "${OUTPUT}" ]] || die 'output already exists'
DARWIN_ARM64="$(checksum git-credit-aarch64-apple-darwin.tar.gz)"
DARWIN_AMD64="$(checksum git-credit-x86_64-apple-darwin.tar.gz)"
LINUX_ARM64="$(checksum git-credit-aarch64-unknown-linux-musl.tar.gz)"
LINUX_AMD64="$(checksum git-credit-x86_64-unknown-linux-musl.tar.gz)"
STAGING="$(mktemp -d "${TMPDIR:-/tmp}/git-credit-homebrew.XXXXXX")"
trap 'rm -rf "${STAGING}"' EXIT
mkdir -p "${STAGING}/Formula"
cat >"${STAGING}/Formula/git-credit.rb" <<EOF
class GitCredit < Formula
  desc "Fair git authorship stats"
  homepage "https://github.com/avhn/git-credit"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/avhn/git-credit/releases/download/v${VERSION}/git-credit-aarch64-apple-darwin.tar.gz"
      sha256 "${DARWIN_ARM64}"
    end
    on_intel do
      url "https://github.com/avhn/git-credit/releases/download/v${VERSION}/git-credit-x86_64-apple-darwin.tar.gz"
      sha256 "${DARWIN_AMD64}"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/avhn/git-credit/releases/download/v${VERSION}/git-credit-aarch64-unknown-linux-musl.tar.gz"
      sha256 "${LINUX_ARM64}"
    end
    on_intel do
      url "https://github.com/avhn/git-credit/releases/download/v${VERSION}/git-credit-x86_64-unknown-linux-musl.tar.gz"
      sha256 "${LINUX_AMD64}"
    end
  end

  def install
    bin.install "git-credit"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/git-credit --version")
  end
end
EOF
mkdir -p "$(dirname "${OUTPUT}")"
mv "${STAGING}" "${OUTPUT}"
