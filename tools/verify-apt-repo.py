#!/usr/bin/env python3
"""Authenticate retained APT bytes before re-indexing or deploying them.

Usage: verify-apt-repo.py REPO_DIR TRUSTED_PUBLIC_KEY EXPECTED_FINGERPRINT
       verify-apt-repo.py --key PUBLIC_KEY EXPECTED_FINGERPRINT
Only source-controlled public keys are trusted, never keys from the retained site.
APT_PREVIOUS_FINGERPRINT optionally enables the adjacent committed
 git-credit-previous-archive-keyring.asc for one reviewed key-rotation transition.
Run with Python -I from the trusted checkout, not from downloaded site content.
"""

import hashlib
import os
import pathlib
import re
import subprocess
import sys
import tempfile


FINGERPRINT = re.compile(r"[0-9A-F]{40}")
DIGEST = re.compile(r"[0-9a-f]{64}")


def run(arguments):
    """Run a bounded public-key operation without exposing command output on failure."""
    result = subprocess.run(arguments, capture_output=True, timeout=60)
    if result.returncode:
        raise ValueError("public key or retained InRelease signature verification failed")
    return result.stdout.decode("utf-8")


def public_key(path, expected, home):
    """Require exactly one primary public key and fingerprint, excluding subkey records."""
    if not FINGERPRINT.fullmatch(expected):
        raise ValueError("invalid expected public key fingerprint")
    if path.is_symlink() or not path.is_file():
        raise ValueError("trusted public key must be a regular committed file")
    listing = run(["gpg", "--no-options", "--homedir", str(home), "--batch",
                   "--with-colons", "--show-keys", str(path)])
    primaries, fingerprints, primary = 0, [], False
    for line in listing.splitlines():
        fields = line.split(":")
        if fields[0] == "pub":
            primaries += 1
            primary = True
        elif fields[0] in ("sub", "sec", "ssb"):
            primary = False
        elif fields[0] == "fpr" and primary:
            fingerprints.append(fields[9])
            primary = False
    if primaries != 1 or fingerprints != [expected]:
        raise ValueError("public keyring must contain exactly one expected primary key")


def paragraphs(text):
    """Parse Debian control paragraphs, rejecting malformed or duplicate field names."""
    result, current, field = [], {}, None
    for line in text.splitlines() + [""]:
        if not line:
            if current:
                result.append(current)
            current, field = {}, None
        elif line.startswith((" ", "\t")):
            if field is None:
                raise ValueError("invalid retained APT control continuation")
            current[field] += "\n" + line.strip()
        else:
            name, separator, value = line.partition(":")
            if not separator or not re.fullmatch(r"[A-Za-z0-9-]+", name) or name in current:
                raise ValueError("invalid or duplicate retained APT control field")
            current[name], field = value.strip(), name
    return result


def verify_file(path, digest, size):
    """Hash one regular file, requiring its signed size and lowercase SHA-256 to match."""
    if not DIGEST.fullmatch(digest) or not re.fullmatch(r"[0-9]+", size):
        raise ValueError("invalid retained SHA-256 record")
    if path.is_symlink() or not path.is_file() or path.stat().st_size != int(size):
        raise ValueError("retained file size or type mismatch: " + path.name)
    with path.open("rb") as source:
        actual = hashlib.sha256()
        for block in iter(lambda: source.read(1024 * 1024), b""):
            actual.update(block)
    if actual.hexdigest() != digest:
        raise ValueError("retained file SHA-256 mismatch: " + path.name)


def verify(repository, key, expected):
    """Verify InRelease, its indexes and every retained package before accepting new uploads."""
    if repository.is_symlink() or (repository.exists() and not repository.is_dir()):
        raise ValueError("invalid retained repository directory")
    if any(path.is_symlink() for path in repository.rglob("*")):
        raise ValueError("repository contains a symlink")
    with tempfile.TemporaryDirectory(prefix="git-credit-apt-verify-") as directory:
        home = pathlib.Path(directory)
        home.chmod(0o700)
        public_key(key, expected, home)
        allowed = {expected}
        keys = [(key, expected)]
        previous = os.environ.get("APT_PREVIOUS_FINGERPRINT", "")
        if previous:
            if previous == expected:
                raise ValueError("previous fingerprint must differ from the current fingerprint")
            previous_key = key.with_name("git-credit-previous-archive-keyring.asc")
            public_key(previous_key, previous, home)
            keys.append((previous_key, previous))
            allowed.add(previous)
        pool = repository / "pool"
        files = set()
        for path in pool.rglob("*"):
            if not path.is_dir():
                if not path.is_file() or path.suffix != ".deb":
                    raise ValueError("unexpected retained pool entry")
                files.add(path.relative_to(repository).as_posix())
        signed = repository / "dists/stable/InRelease"
        if not signed.exists():
            if files:
                raise ValueError("unsigned retained pool must be empty before first publication")
            return
        command = ["gpgv", "--homedir", str(home), "--status-fd", "1"]
        for index, (source, _) in enumerate(keys):
            target = home / (str(index) + ".gpg")
            run(["gpg", "--no-options", "--homedir", str(home), "--batch", "--yes",
                 "--output", str(target), "--dearmor", str(source)])
            command += ["--keyring", str(target)]
        plaintext = home / "Release"
        status = run(command + ["--output", str(plaintext), str(signed)])
        signatures = [line.split()[-1] for line in status.splitlines()
                      if line.startswith("[GNUPG:] VALIDSIG ")]
        if len(signatures) != 1 or signatures[0] not in allowed:
            raise ValueError("retained InRelease signer fingerprint mismatch")
        for suffix in ("gpg", "asc"):
            public_key(repository / ("git-credit-archive-keyring." + suffix), signatures[0], home)
        releases = paragraphs(plaintext.read_text(encoding="utf-8"))
        if len(releases) != 1 or "SHA256" not in releases[0]:
            raise ValueError("retained InRelease must contain one SHA256 manifest")
        hashes = {}
        for line in releases[0]["SHA256"].splitlines():
            if not line.strip():
                continue
            fields = line.split()
            if len(fields) != 3 or fields[2] in hashes:
                raise ValueError("invalid or duplicate retained index checksum")
            digest, size, name = fields
            hashes[name] = (digest, size)
        packages = {}
        for architecture in ("amd64", "arm64"):
            relative = "main/binary-" + architecture + "/Packages"
            for name in (relative, relative + ".gz"):
                if name not in hashes:
                    raise ValueError("missing signed retained Packages index")
                verify_file(repository / "dists/stable" / name, *hashes[name])
            for entry in paragraphs((repository / "dists/stable" / relative).read_text(encoding="utf-8")):
                name = entry.get("Filename", "")
                if (entry.get("Package") != "git-credit" or entry.get("Architecture") != architecture
                        or not re.fullmatch(r"pool/main/g/git-credit/git-credit_[0-9][A-Za-z0-9_.+~-]*_" + architecture + r"\.deb", name)
                        or name in packages or "SHA256" not in entry or "Size" not in entry):
                    raise ValueError("invalid or duplicate retained package record")
                packages[name] = (entry["SHA256"], entry["Size"])
        if files != set(packages):
            raise ValueError("unlisted or missing retained Debian package")
        for name, (digest, size) in packages.items():
            verify_file(repository / name, digest, size)


if __name__ == "__main__":
    try:
        if len(sys.argv) == 4 and sys.argv[1] == "--key":
            with tempfile.TemporaryDirectory(prefix="git-credit-public-key-") as directory:
                home = pathlib.Path(directory)
                home.chmod(0o700)
                public_key(pathlib.Path(sys.argv[2]), sys.argv[3], home)
        elif len(sys.argv) == 4:
            verify(pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2]), sys.argv[3])
        else:
            raise ValueError("usage: verify-apt-repo.py REPO_DIR PUBLIC_KEY FINGERPRINT; or --key PUBLIC_KEY FINGERPRINT")
    except (ValueError, OSError, UnicodeError, subprocess.TimeoutExpired) as error:
        sys.exit("git-credit retained APT verification failed: " + str(error))
