#!/bin/sh
# Install git-credit from its GitHub release on macOS or Linux, without sudo.
#
#   curl -fsSL https://raw.githubusercontent.com/avhn/git-credit/main/install.sh | sh
#
# Environment:
#   GIT_CREDIT_VERSION      release to install, for example 0.1.1 (default: latest)
#   GIT_CREDIT_INSTALL_DIR  target directory (default: $HOME/.local/bin)
#
# The archive is verified against the release's SHA256SUMS before anything is
# installed. A checksum detects a corrupted download; it cannot detect a
# compromised release account. Package managers (Homebrew, APT) are documented in
# the README for users who prefer signed repositories and automatic upgrades.
set -eu

repo="avhn/git-credit"
install_dir="${GIT_CREDIT_INSTALL_DIR:-$HOME/.local/bin}"

# die MESSAGE: print an error to stderr and stop.
die() {
    printf 'git-credit install: %s\n' "$1" >&2
    exit 1
}

command -v curl >/dev/null 2>&1 || die "curl is required"
command -v tar >/dev/null 2>&1 || die "tar is required"

# Map the host to a published release target.
os="$(uname -s)"
arch="$(uname -m)"
case "$arch" in
    x86_64 | amd64) arch="x86_64" ;;
    arm64 | aarch64) arch="aarch64" ;;
    *) die "unsupported CPU architecture: $arch" ;;
esac
case "$os" in
    Darwin) target="$arch-apple-darwin" ;;
    Linux) target="$arch-unknown-linux-musl" ;;
    *) die "unsupported operating system: $os (Windows: use install.ps1)" ;;
esac

# Resolve the release tag: an explicit version, or the latest release redirect.
if [ -n "${GIT_CREDIT_VERSION:-}" ]; then
    tag="v${GIT_CREDIT_VERSION#v}"
else
    latest="$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$repo/releases/latest")" ||
        die "cannot reach GitHub to find the latest release"
    tag="${latest##*/}"
fi
case "$tag" in
    v[0-9]*.[0-9]*.[0-9]*) ;;
    *) die "could not determine a release version (got '$tag')" ;;
esac

# Pick a SHA-256 tool that exists on this host.
if command -v sha256sum >/dev/null 2>&1; then
    sha256() { sha256sum "$1" | cut -d ' ' -f 1; }
elif command -v shasum >/dev/null 2>&1; then
    sha256() { shasum -a 256 "$1" | cut -d ' ' -f 1; }
else
    die "sha256sum or shasum is required to verify the download"
fi

archive="git-credit-$target.tar.gz"
base="https://github.com/$repo/releases/download/$tag"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

printf 'Downloading git-credit %s for %s\n' "$tag" "$target"
curl -fsSL -o "$tmp/$archive" "$base/$archive" || die "download failed: $base/$archive"
curl -fsSL -o "$tmp/SHA256SUMS" "$base/SHA256SUMS" || die "download failed: $base/SHA256SUMS"

expected="$(awk -v name="$archive" '$2 == name || $2 == "*" name { print $1; exit }' "$tmp/SHA256SUMS")"
[ -n "$expected" ] || die "$archive is not listed in SHA256SUMS"
actual="$(sha256 "$tmp/$archive")"
[ "$expected" = "$actual" ] || die "checksum mismatch for $archive"

tar -xzf "$tmp/$archive" -C "$tmp" git-credit || die "archive does not contain git-credit"
mkdir -p "$install_dir"
# Write next to the target, then rename, so a running copy is never half-written.
cp "$tmp/git-credit" "$install_dir/.git-credit.new"
chmod 0755 "$install_dir/.git-credit.new"
mv -f "$install_dir/.git-credit.new" "$install_dir/git-credit"

printf 'Installed %s to %s\n' "$("$install_dir/git-credit" --version)" "$install_dir/git-credit"
case ":$PATH:" in
    *":$install_dir:"*) printf 'Try it inside any git repository: git credit analyze\n' ;;
    *)
        printf '\n%s is not on your PATH. Add this line to your shell profile:\n' "$install_dir"
        # shellcheck disable=SC2016 # $PATH is printed literally for the user's profile.
        printf '  export PATH="%s:$PATH"\n' "$install_dir"
        ;;
esac
