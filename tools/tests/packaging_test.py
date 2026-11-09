#!/usr/bin/env python3
"""Test formula rendering, retained site guards and append-only tap documentation offline."""

import importlib.util
import os
import pathlib
import shutil
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("tap_readme", ROOT / "tools/update-tap-readme.py")
TAP = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(TAP)


class Packaging(unittest.TestCase):
    """Use inert checksums and temporary site content, without Git or network publication."""

    def setUp(self):
        """Create independent input and output paths, including spaces."""
        self.temp = tempfile.TemporaryDirectory(prefix="git-credit packaging ")
        self.addCleanup(self.temp.cleanup)
        self.root = pathlib.Path(self.temp.name)
        self.sums = self.root / "SHA256SUMS"
        self.sums.write_bytes((ROOT / "tools/tests/fixtures/SHA256SUMS").read_bytes())

    def script(self, script, *args, diagnostic=None):
        """Run a trusted script from the trusted checkout and require its expected outcome."""
        result = subprocess.run(["bash", str(ROOT / "tools" / script), *map(str, args)],
                                cwd=ROOT, capture_output=True, text=True, timeout=30)
        if diagnostic is None:
            self.assertEqual(result.returncode, 0, result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0, result.stdout)
            self.assertIn(diagnostic, result.stderr)
        return result

    def test_four_platform_formula_and_repeatability(self):
        """Render each exact URL/hash, the dual license, binary install and version test."""
        self.script("render-homebrew.sh", "v0.1.1", self.sums, self.root / "first")
        self.script("render-homebrew.sh", "0.1.1", self.sums, self.root / "second")
        formula = self.root / "first/Formula/git-credit.rb"
        self.assertEqual(formula.read_bytes(), (self.root / "second/Formula/git-credit.rb").read_bytes())
        text = formula.read_text()
        for line in self.sums.read_text().splitlines():
            digest, asset = line.split()
            self.assertIn("https://github.com/avhn/git-credit/releases/download/v0.1.1/" + asset, text)
            self.assertIn('sha256 "' + digest + '"', text)
        for fragment in ('license any_of: ["MIT", "Apache-2.0"]', 'bin.install "git-credit"',
                         'assert_match version.to_s, shell_output("#{bin}/git-credit --version")'):
            self.assertIn(fragment, text)
        if shutil.which("ruby"):
            result = subprocess.run(["ruby", "-c", str(formula)], cwd=ROOT,
                                    capture_output=True, text=True, timeout=30)
            self.assertEqual(result.returncode, 0, result.stderr)
            print(result.stdout.strip())
        else:
            print("SKIP: ruby -c (Ruby is unavailable)")

    def test_renderer_rejects_bad_inputs_without_partial_output(self):
        """Reject injection, prereleases, occupied output and ambiguous or malformed hashes."""
        output = self.root / "output"
        for version in ('0.1.1-rc.1', '0.1.1;false'):
            self.script("render-homebrew.sh", version, self.sums, output,
                        diagnostic="invalid stable release version")
        original = self.sums.read_text()
        for text, error in ((original + original, "missing or duplicate checksum"),
                            ("\n".join(original.splitlines()[1:]), "missing or duplicate checksum"),
                            (original.replace("1" * 64, "z" * 64), "invalid checksum")):
            self.sums.write_text(text)
            self.script("render-homebrew.sh", "0.1.1", self.sums, output, diagnostic=error)
            self.assertFalse(output.exists())
        self.sums.write_text(original)
        output.mkdir()
        self.script("render-homebrew.sh", "0.1.1", self.sums, output,
                    diagnostic="output already exists")

    def test_tap_readme_is_append_only_and_idempotent(self):
        """Keep fortix's existing text and add both install commands exactly once."""
        readme = self.root / "README.md"
        original = "# homebrew-tap\nHomebrew tap for fortix, an open source FortiGate SSL VPN client\n\n\n"
        readme.write_text(original)
        TAP.update(readme)
        first = readme.read_bytes()
        TAP.update(readme)
        self.assertEqual(readme.read_bytes(), first)
        self.assertTrue(readme.read_text().startswith(original))
        for name in ("fortix", "git-credit"):
            self.assertEqual(readme.read_text().count("brew install avhn/tap/" + name), 1)

    def test_site_retains_apt_bytes_and_checks_fingerprint(self):
        """The site renderer only updates index.html, never retained signed package metadata."""
        site = self.root / "site"
        (site / "apt").mkdir(parents=True)
        retained = site / "apt/InRelease"
        retained.write_text("retained signed bytes")
        fingerprint = "097DDFE92CE4B0F4E204F434668F8E1D756456FD"
        self.script("build-pages-index.sh", site, fingerprint)
        text = (site / "index.html").read_text()
        for fragment in (fingerprint, "brew install avhn/tap/git-credit", "https://avhn.github.io/git-credit/apt stable main",
                         "/usr/share/keyrings/git-credit-archive-keyring.gpg"):
            self.assertIn(fragment, text)
        self.assertEqual(retained.read_text(), "retained signed bytes")
        self.script("build-pages-index.sh", site, "unsafe", diagnostic="invalid signing key fingerprint")

    def test_missing_publication_secrets_fail_before_network(self):
        """An empty required secret is an actionable error, not a silently skipped job."""
        for missing in ("GH_TOKEN", "APT_SIGNING_KEY", "APT_SIGNING_PASSPHRASE"):
            env = os.environ.copy()
            env.update(GH_TOKEN="inert fixture", APT_SIGNING_KEY="inert fixture",
                       APT_SIGNING_PASSPHRASE="inert fixture")
            env[missing] = ""
            result = subprocess.run(["bash", str(ROOT / "tools/prepare-pages.sh"),
                                     "release", str(self.root), "v0.1.1"], cwd=ROOT, env=env,
                                    capture_output=True, text=True, timeout=30)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(missing + " is required", result.stderr)


if __name__ == "__main__":
    unittest.main()
