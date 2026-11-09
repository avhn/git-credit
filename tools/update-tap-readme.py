#!/usr/bin/env python3
"""Append installation entries without removing any existing tap README content."""

import pathlib
import sys


def update(path):
    """Add only missing commands, preserving fortix and making repeated releases a no-op."""
    if path.is_symlink():
        raise ValueError("tap README must not be a symlink")
    original = path.read_text(encoding="utf-8") if path.exists() else "# Homebrew tap\n"
    text = original
    for command, description in (
        ("brew install avhn/tap/fortix", "fortix: FortiGate SSL VPN client"),
        ("brew install avhn/tap/git-credit", "git-credit: fair git authorship stats"),
    ):
        if command not in text:
            separator = "\n" if text.endswith("\n") else "\n\n"
            text += separator + description + "\n\n```sh\n" + command + "\n```\n"
    if text != original or not path.exists():
        path.write_text(text, encoding="utf-8", newline="\n")


if __name__ == "__main__":
    update(pathlib.Path(sys.argv[1]))
