#!/usr/bin/env python3
"""Verify a release checksum and copy only its regular binary into an empty directory.

Archives are untrusted. Never extract paths, links, scripts or other archive members.
The caller runs Python in isolated mode and keeps downloaded data outside this file.
"""

import hashlib
import pathlib
import re
import sys
import tarfile
import zipfile


def checksum(archive, checksums):
    """Verify one regular payload against exactly one manifest entry without extraction."""
    if archive.is_symlink() or not archive.is_file():
        raise ValueError("release payload must be a regular file")
    matches = []
    for line in checksums.read_text(encoding="utf-8").splitlines():
        match = re.fullmatch(r"([0-9a-fA-F]{64}) [ *](.+)", line)
        if match and match[2] == archive.name:
            matches.append(match[1].lower())
    if len(matches) != 1:
        raise ValueError("expected exactly one SHA256SUMS entry for " + archive.name)
    with archive.open("rb") as source:
        digest = hashlib.sha256()
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    if digest.hexdigest() != matches[0]:
        raise ValueError("release checksum mismatch for " + archive.name)


def provenance(trusted, downloaded, expected):
    """Require release bytes to match the publish job's manifest artifact, not release metadata."""
    manifest = downloaded / "SHA256SUMS"
    if (trusted.is_symlink() or not trusted.is_file() or manifest.is_symlink()
            or not manifest.is_file() or trusted.read_bytes() != manifest.read_bytes()):
        raise ValueError("release SHA256SUMS differs from the publish job artifact")
    if any(not re.fullmatch(r"[A-Za-z0-9_.+-]+", name) for name in expected):
        raise ValueError("unsafe expected release asset name")
    actual = {path.name for path in downloaded.iterdir() if path.name != "SHA256SUMS"}
    if not expected or len(expected) != len(set(expected)) or actual != set(expected):
        raise ValueError("incomplete or unexpected downloaded release asset set")
    for name in expected:
        checksum(downloaded / name, trusted)


def select_asset(checksums, target):
    """Prefer musl on Linux, with GNU fallback only when musl is absent from the manifest."""
    allowed = ("x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl",
               "x86_64-apple-darwin", "aarch64-apple-darwin", "x86_64-pc-windows-msvc")
    if target not in allowed:
        raise ValueError("unsupported release target")
    extension = ".zip" if "windows" in target else ".tar.gz"
    candidates = ["git-credit-" + target + extension]
    if target.endswith("-musl"):
        candidates.append("git-credit-" + target.removesuffix("-musl") + "-gnu" + extension)
    entries = []
    for line in checksums.read_text(encoding="utf-8").splitlines():
        fields = line.split()
        if fields:
            basename = fields[1].removeprefix("*") if len(fields) >= 2 else fields[0]
            entries.append((basename, line))
    for name in candidates:
        matching = [line for basename, line in entries if basename == name]
        if matching:
            if len(matching) != 1 or not re.fullmatch(r"[0-9a-fA-F]{64} [ *]" + re.escape(name), matching[0]):
                raise ValueError("invalid or duplicate release checksum for " + name)
            return name
    raise ValueError("no release asset for " + target)


def verify(archive, checksums, destination, executable):
    """Fail closed on missing/duplicate checksums, nonregular binaries or occupied output."""
    checksum(archive, checksums)
    if destination.is_symlink() or not destination.is_dir() or any(destination.iterdir()):
        raise ValueError("binary destination must be a new empty directory")
    if executable not in ("git-credit", "git-credit.exe"):
        raise ValueError("unexpected executable name")
    if archive.suffix == ".zip":
        with zipfile.ZipFile(archive) as package:
            members = [item for item in package.infolist() if item.filename == executable]
            if len(members) != 1 or members[0].is_dir():
                raise ValueError("expected exactly one regular release binary")
            mode = members[0].external_attr >> 16
            if mode & 0o170000 not in (0, 0o100000):
                raise ValueError("release binary is not a regular file")
            data = package.read(members[0])
    else:
        with tarfile.open(archive, "r:gz") as package:
            members = [item for item in package.getmembers() if item.name == executable]
            if len(members) != 1 or not members[0].isfile():
                raise ValueError("expected exactly one regular release binary")
            with package.extractfile(members[0]) as source:
                data = source.read()
    output = destination / executable
    with output.open("xb") as target:
        target.write(data)
    output.chmod(0o755)


if __name__ == "__main__":
    try:
        if len(sys.argv) >= 5 and sys.argv[1] == "--provenance":
            provenance(pathlib.Path(sys.argv[2]), pathlib.Path(sys.argv[3]), sys.argv[4:])
        elif len(sys.argv) == 4 and sys.argv[1] == "--select":
            print(select_asset(pathlib.Path(sys.argv[2]), sys.argv[3]))
        elif len(sys.argv) == 4 and sys.argv[1] == "--checksum-only":
            checksum(pathlib.Path(sys.argv[2]), pathlib.Path(sys.argv[3]))
        elif len(sys.argv) == 5:
            verify(*(pathlib.Path(arg) for arg in sys.argv[1:4]), sys.argv[4])
        else:
            raise ValueError("usage: verify-release.py ARCHIVE SHA256SUMS DESTINATION EXECUTABLE; or --select SHA256SUMS TARGET; or --checksum-only PAYLOAD SHA256SUMS; or --provenance TRUSTED_SHA256SUMS DOWNLOAD_DIR ASSET...")
    except (ValueError, OSError, tarfile.TarError, zipfile.BadZipFile) as error:
        sys.exit("git-credit release verification failed: " + str(error))
