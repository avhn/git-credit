#!/usr/bin/env python3
"""Package a built target binary, licenses and README with reproducible metadata."""

import gzip
import io
import pathlib
import sys
import tarfile
import zipfile


def package(target):
    """Write one flat archive with fixed timestamps, modes and deterministic ordering."""
    windows = target == "x86_64-pc-windows-msvc"
    executable = "git-credit.exe" if windows else "git-credit"
    inputs = [(executable, pathlib.Path("target") / target / "release" / executable)]
    inputs += [(name, pathlib.Path(name)) for name in ("LICENSE-APACHE", "LICENSE-MIT", "README.md")]
    destination = pathlib.Path("dist")
    destination.mkdir(exist_ok=True)
    output = destination / ("git-credit-" + target + (".zip" if windows else ".tar.gz"))
    if windows:
        with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED) as archive:
            for name, path in inputs:
                info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
                info.compress_type = zipfile.ZIP_DEFLATED
                info.external_attr = (0o100755 if name == executable else 0o100644) << 16
                archive.writestr(info, path.read_bytes())
    else:
        with output.open("wb") as raw, gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed, tarfile.open(fileobj=compressed, mode="w") as archive:
            for name, path in inputs:
                data = path.read_bytes()
                info = tarfile.TarInfo(name)
                info.size = len(data)
                info.mode = 0o755 if name == executable else 0o644
                archive.addfile(info, io.BytesIO(data))
    print(output)


if __name__ == "__main__":
    package(sys.argv[1])
