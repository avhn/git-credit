#!/usr/bin/env bash
# Resolve a local binary or verify and extract exactly one release executable.
set -euo pipefail

python_cmd=python3
if [ "${RUNNER_OS:?}" = Windows ]; then python_cmd=python; fi
if [ -n "${GC_BINARY:-}" ]; then
    binary="$($python_cmd -I -c 'import os, sys; print(os.path.abspath(sys.argv[1]))' "$GC_BINARY")"
    if [ ! -f "$binary" ]; then
        echo "git-credit binary does not exist: $binary" >&2
        exit 1
    fi
else
    case "$RUNNER_OS/${RUNNER_ARCH:?}" in
        Linux/X64) target=x86_64-unknown-linux-musl ;;
        Linux/ARM64) target=aarch64-unknown-linux-musl ;;
        macOS/X64) target=x86_64-apple-darwin ;;
        macOS/ARM64) target=aarch64-apple-darwin ;;
        Windows/X64) target=x86_64-pc-windows-msvc ;;
        *)
            echo "Unsupported release runner: $RUNNER_OS/$RUNNER_ARCH" >&2
            exit 1
            ;;
    esac
    version="${GC_VERSION:-latest}"
    if [ "$version" = latest ]; then
        base=https://github.com/avhn/git-credit/releases/latest/download
    elif [[ "$version" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9.-]+)?$ ]]; then
        base="https://github.com/avhn/git-credit/releases/download/$version"
    else
        echo 'version must be latest or a v-prefixed release tag such as v0.1.0' >&2
        exit 1
    fi
    executable=git-credit
    if [ "$RUNNER_OS" = Windows ]; then executable=git-credit.exe; fi
    # Downloaded data and the extracted executable each get a new empty directory.
    download="$(mktemp -d "${RUNNER_TEMP:?}/git-credit-download.XXXXXX")"
    install="$(mktemp -d "$RUNNER_TEMP/git-credit-bin.XXXXXX")"
    trap 'rm -rf -- "$download"' EXIT
    curl --proto '=https' --tlsv1.2 -fsSL --retry 3 --connect-timeout 20 --max-time 60 "$base/SHA256SUMS" -o "$download/SHA256SUMS"
    # Fall back to v0.1.0 GNU names only when the manifest has no musl entry.
    archive="$("$python_cmd" -I "$GITHUB_ACTION_PATH/tools/verify-release.py" --select "$download/SHA256SUMS" "$target")"
    curl --proto '=https' --tlsv1.2 -fsSL --retry 3 --connect-timeout 20 --max-time 300 "$base/$archive" -o "$download/$archive"
    "$python_cmd" -I "$GITHUB_ACTION_PATH/tools/verify-release.py" "$download/$archive" "$download/SHA256SUMS" "$install" "$executable"
    binary="$install/$executable"
fi
if [[ "$binary" = *$'\n'* || "$binary" = *$'\r'* ]]; then
    echo 'Binary path contains a newline' >&2
    exit 1
fi
printf 'binary=%s\n' "$binary" >>"${GITHUB_OUTPUT:?}"
