#!/usr/bin/env python3
"""Copy only recognized regular generated assets into a fresh publication directory."""

import pathlib
import re
import shutil
import sys


def stage(source, destination):
    """Refuse symlinks and exclude unrelated files, including credentials and source."""
    for path in (source, *source.parents):
        if path.is_symlink():
            raise ValueError("badge output must not traverse symlinks")
    if not source.is_dir():
        raise ValueError("badge output directory does not exist")
    if destination.is_symlink() or any(destination.iterdir()):
        raise ValueError("publication directory must be new and empty")
    required = ["credit.json", "bus-factor.json", "top-contributor.json"]
    files = [source / name for name in required]
    chart_names = ("credit-bar", "credit-card", "credit-donut", "credit-labor", "credit-treemap", "bus-factor")
    files += [source / (name + suffix + ".svg")
              for name in chart_names for suffix in ("", "-dark")
              if (source / (name + suffix + ".svg")).exists()]
    authors = source / "authors"
    if authors.is_symlink():
        raise ValueError("authors directory must not be a symlink")
    if authors.is_dir():
        files += sorted(path for path in authors.iterdir() if re.fullmatch(r"author-[a-z0-9-]+\.svg", path.name))
    for path in files:
        if path.is_symlink() or not path.is_file():
            raise ValueError("expected a regular generated asset: " + str(path))
    for path in files:
        output = destination / path.relative_to(source)
        output.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(path, output)


if __name__ == "__main__":
    try:
        stage(pathlib.Path(sys.argv[1]).absolute(), pathlib.Path(sys.argv[2]))
    except (ValueError, OSError) as error:
        sys.exit("git-credit staging failed: " + str(error))
