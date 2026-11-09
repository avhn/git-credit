#!/usr/bin/env python3
"""Test strict public-key cardinality and retained control/hash parsing without network or secrets."""

import hashlib
import importlib.util
import pathlib
import tempfile
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("apt_verifier", ROOT / "tools/verify-apt-repo.py")
APT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(APT)
FINGERPRINT = "097DDFE92CE4B0F4E204F434668F8E1D756456FD"


class AptVerifier(unittest.TestCase):
    """Isolate metadata parsing from GPG, with real signed-byte integration in packaging.test.sh."""

    def test_exactly_one_primary_key_ignores_subkeys(self):
        """Accept one expected primary with a subkey, but reject additional primary keys."""
        primary = "pub:::::::::\n" + "fpr:::::::::" + FINGERPRINT + ":\n"
        subkey = "sub:::::::::\n" + "fpr:::::::::" + "A" * 40 + ":\n"
        with tempfile.TemporaryDirectory() as directory:
            home = pathlib.Path(directory)
            key = home / "public.asc"
            key.write_text("inert public-key fixture, not parsed by real GPG")
            for listing in (primary, primary + subkey):
                with mock.patch.object(APT, "run", return_value=listing):
                    APT.public_key(key, FINGERPRINT, home)
            for listing in (primary + primary, primary + primary.replace(FINGERPRINT, "B" * 40),
                            "", "pub:::::::::\n", primary.replace(FINGERPRINT, "B" * 40)):
                with mock.patch.object(APT, "run", return_value=listing):
                    with self.assertRaisesRegex(ValueError, "exactly one expected primary key"):
                        APT.public_key(key, FINGERPRINT, home)

    def test_control_paragraphs_and_duplicate_fields(self):
        """Preserve SHA256 continuations while rejecting ambiguous Debian control records."""
        text = "SHA256:\n abc 1 path\n def 2 other\nSuite: stable\n"
        self.assertEqual(APT.paragraphs(text), [{"SHA256": "\nabc 1 path\ndef 2 other", "Suite": "stable"}])
        for text in ("Package: first\nPackage: second\n", " orphan continuation\n", "missing separator\n"):
            with self.assertRaises(ValueError):
                APT.paragraphs(text)

    def test_retained_file_hash_and_size_must_match(self):
        """Reject changed bytes even when a package keeps its original filename and size."""
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "retained.deb"
            path.write_bytes(b"good")
            digest = hashlib.sha256(b"good").hexdigest()
            APT.verify_file(path, digest, "4")
            path.write_bytes(b"evil")
            with self.assertRaisesRegex(ValueError, "SHA-256 mismatch"):
                APT.verify_file(path, digest, "4")
            with self.assertRaisesRegex(ValueError, "size or type mismatch"):
                APT.verify_file(path, digest, "5")


if __name__ == "__main__":
    unittest.main()
