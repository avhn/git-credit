#!/usr/bin/env bash
# Render the static installation page beside the retained APT repository.
# Usage: build-pages-index.sh SITE_DIR KEY_FINGERPRINT
# SITE_DIR must exist and contain no symlinks, including existing output files.
set -euo pipefail

# die refuses unsafe site paths before redirecting generated HTML.
die() {
    printf '%s\n' "${*}" >&2
    exit 1
}

[[ $# -eq 2 ]] || die 'usage: build-pages-index.sh SITE_DIR KEY_FINGERPRINT'
FINGERPRINT="${2}"
[[ "${FINGERPRINT}" =~ ^[0-9A-F]{40}$ ]] || die 'invalid signing key fingerprint'
[[ -d "${1}" && ! -L "${1}" ]] || die 'invalid site directory'
SITE="$(cd "${1}" && pwd)"
[[ -z "$(find "${SITE}" -type l -print -quit)" ]] || die 'site contains a symlink'
cat >"${SITE}/index.html" <<EOF
<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>git-credit: fair git authorship stats</title>
</head>
<body>
<main>
<h1>git-credit</h1>
<p>Fair git authorship stats. Ownership describes surviving meaningful code;
 labor includes meaningful historical work. Neither measures a person's value.</p>
<p><a href="https://github.com/avhn/git-credit">Source and documentation</a> |
<a href="https://github.com/avhn/git-credit/releases/latest">Binaries and Debian packages</a></p>
<h2>Homebrew</h2>
<pre>brew install avhn/tap/git-credit</pre>
<h2>Debian 11+ and Ubuntu 20.04+</h2>
<p>Signed packages for amd64 and arm64. Signing key fingerprint:
<code>${FINGERPRINT}</code>. Compare it with the independently reviewed source documentation.</p>
<pre>sudo apt update &amp;&amp; sudo apt install -y ca-certificates curl gnupg
curl -fsSL https://avhn.github.io/git-credit/apt/git-credit-archive-keyring.gpg -o git-credit-archive-keyring.gpg
gpg --show-keys --with-fingerprint git-credit-archive-keyring.gpg
# Require exactly one primary public key with the fingerprint above before installing it.
sudo install -m 0644 git-credit-archive-keyring.gpg /usr/share/keyrings/git-credit-archive-keyring.gpg
echo 'deb [signed-by=/usr/share/keyrings/git-credit-archive-keyring.gpg] https://avhn.github.io/git-credit/apt stable main' | sudo tee /etc/apt/sources.list.d/git-credit.list
sudo apt update &amp;&amp; sudo apt install git-credit</pre>
<p><a href="apt/dists/stable/InRelease">Signed repository index</a> |
<a href="apt/git-credit-archive-keyring.asc">Armored public key</a></p>
</main>
</body>
</html>
EOF
