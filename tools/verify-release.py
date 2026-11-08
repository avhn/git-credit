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


def verify(archive, checksums, destination, executable):
    """Fail closed on missing/duplicate checksums, nonregular binaries or occupied output."""
    matches = []
    for line in checksums.read_text(encoding="utf-8").splitlines():
        match = re.fullmatch(r"([0-9a-fA-F]{64}) [ *](.+)", line)
        if match and match[2] == archive.name:
            matches.append(match[1].lower())
    if len(matches) != 1:
        raise ValueError("expected exactly one SHA256SUMS entry for " + archive.name)
    with archive.open("rb") as source:
        digest = hashlib.file_digest(source, "sha256").hexdigest()
    if digest != matches[0]:
        raise ValueError("release checksum mismatch for " + archive.name)
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
        verify(*(pathlib.Path(arg) for arg in sys.argv[1:4]), sys.argv[4])
    except (ValueError, OSError, tarfile.TarError, zipfile.BadZipFile) as error:
        sys.exit("git-credit release verification failed: " + str(error))
