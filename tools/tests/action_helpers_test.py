#!/usr/bin/env python3
"""Exercise checksum/extraction, publication staging and mocked lease/freshness paths.

All network and publishing Git commands are mocked. No token, remote, commit or
push is used. Fixture archives and output directories are private temporary data.
"""

import hashlib
import importlib.util
import io
import os
import pathlib
import shutil
import subprocess
import tarfile
import tempfile
import unittest
import zipfile

ROOT = pathlib.Path(__file__).resolve().parents[2]


def load(name, filename):
    """Import a trusted repository helper by exact path under isolated Python."""
    spec = importlib.util.spec_from_file_location(name, ROOT / "tools" / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


VERIFY = load("verify_release", "verify-release.py")
STAGE = load("stage_badges", "stage-badges.py")
PACKAGE = load("package_release", "package-release.py")


class Helpers(unittest.TestCase):
    """Test archive and staging invariants without relying on external services."""

    def setUp(self):
        """Give every test independent empty download, extraction and stage paths."""
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = pathlib.Path(self.temp.name)
        self.destination = self.root / "binary"
        self.destination.mkdir()

    def archive(self, kind="tar", link=False):
        """Create an archive with a binary plus an untrusted traversal member."""
        archive = self.root / ("release.zip" if kind == "zip" else "release.tar.gz")
        if kind == "zip":
            with zipfile.ZipFile(archive, "w") as package:
                item = zipfile.ZipInfo("git-credit.exe")
                item.external_attr = (0o120777 if link else 0o100755) << 16
                package.writestr(item, b"fixture binary")
                package.writestr("../outside", b"do not extract")
        else:
            with tarfile.open(archive, "w:gz") as package:
                item = tarfile.TarInfo("git-credit")
                if link:
                    item.type = tarfile.SYMTYPE
                    item.linkname = "../outside"
                    package.addfile(item)
                else:
                    item.size = len(b"fixture binary")
                    package.addfile(item, io.BytesIO(b"fixture binary"))
                item = tarfile.TarInfo("../outside")
                item.size = 1
                package.addfile(item, io.BytesIO(b"x"))
        checksums = self.root / "SHA256SUMS"
        checksums.write_text(hashlib.sha256(archive.read_bytes()).hexdigest() + "  " + archive.name + "\n")
        return archive, checksums

    def test_verified_tar_and_zip_copy_only_binary(self):
        """Ignore archive paths entirely and copy only the selected regular member."""
        for kind, executable in [("tar", "git-credit"), ("zip", "git-credit.exe")]:
            archive, checksums = self.archive(kind)
            destination = self.destination / kind
            destination.mkdir()
            VERIFY.verify(archive, checksums, destination, executable)
            self.assertEqual((destination / executable).read_bytes(), b"fixture binary")
            self.assertEqual(len(list(destination.iterdir())), 1)
            self.assertFalse((self.root / "outside").exists())

    def test_checksum_corruption_duplicates_and_links_fail_closed(self):
        """Reject corrupt or ambiguous sums and both tar and zip symlink binaries."""
        archive, checksums = self.archive()
        good = checksums.read_text()
        checksums.write_text("0" * 64 + "  " + archive.name + "\n")
        with self.assertRaisesRegex(ValueError, "mismatch"):
            VERIFY.verify(archive, checksums, self.destination, "git-credit")
        checksums.write_text(good + good)
        with self.assertRaisesRegex(ValueError, "exactly one SHA256SUMS"):
            VERIFY.verify(archive, checksums, self.destination, "git-credit")
        for kind, executable in [("tar", "git-credit"), ("zip", "git-credit.exe")]:
            archive, checksums = self.archive(kind, link=True)
            with self.assertRaisesRegex(ValueError, "regular"):
                VERIFY.verify(archive, checksums, self.destination, executable)
        self.assertEqual(list(self.destination.iterdir()), [])

    def test_publication_provenance_rejects_replaced_release_manifest_and_payload(self):
        """Trust the publish-job artifact even if release assets and their manifest agree."""
        downloaded = self.root / "release-assets"
        downloaded.mkdir()
        asset = downloaded / "git-credit_0.1.1-1_amd64.deb"
        asset.write_bytes(b"built bytes")
        manifest = downloaded / "SHA256SUMS"
        manifest.write_text(hashlib.sha256(asset.read_bytes()).hexdigest() + "  " + asset.name + "\n")
        trusted = self.root / "trusted-SHA256SUMS"
        trusted.write_bytes(manifest.read_bytes())
        VERIFY.provenance(trusted, downloaded, [asset.name])
        asset.write_bytes(b"replaced release bytes")
        with self.assertRaisesRegex(ValueError, "release checksum mismatch"):
            VERIFY.provenance(trusted, downloaded, [asset.name])
        manifest.write_text(hashlib.sha256(asset.read_bytes()).hexdigest() + "  " + asset.name + "\n")
        with self.assertRaisesRegex(ValueError, "differs from the publish job artifact"):
            VERIFY.provenance(trusted, downloaded, [asset.name])
        manifest.write_bytes(trusted.read_bytes())
        with self.assertRaisesRegex(ValueError, "asset set"):
            VERIFY.provenance(trusted, downloaded, [asset.name, "missing.deb"])

    def test_linux_assets_prefer_musl_and_fall_back_to_legacy_gnu(self):
        """Manifest selection never downgrades a malformed musl entry to a GNU download."""
        sums = self.root / "SHA256SUMS"
        for arch in ("x86_64", "aarch64"):
            target = arch + "-unknown-linux-musl"
            musl = "git-credit-" + target + ".tar.gz"
            gnu = musl.replace("-musl", "-gnu")
            legacy = "a" * 64 + "  " + gnu + "\n"
            modern = "b" * 64 + "  " + musl + "\n"
            for manifest, expected in ((legacy, gnu), (modern, musl), (legacy + modern, musl), ("\n" + legacy + "\n" + modern, musl)):
                sums.write_text(manifest)
                self.assertEqual(VERIFY.select_asset(sums, target), expected)
            for manifest in (legacy + modern + modern, legacy + modern.replace("b" * 64, "invalid"),
                             legacy + modern.rstrip() + " extra\n"):
                sums.write_text(manifest)
                with self.assertRaisesRegex(ValueError, "invalid or duplicate"):
                    VERIFY.select_asset(sums, target)
            sums.write_text("")
            with self.assertRaisesRegex(ValueError, "no release asset"):
                VERIFY.select_asset(sums, target)
        for target, extension in (("aarch64-apple-darwin", ".tar.gz"), ("x86_64-pc-windows-msvc", ".zip")):
            name = "git-credit-" + target + extension
            sums.write_text("a" * 64 + "  " + name + "\n")
            self.assertEqual(VERIFY.select_asset(sums, target), name)

    def test_stage_copies_only_known_regular_assets(self):
        """Do not publish source, dotfiles, arbitrary SVGs or unsafe author paths."""
        source = self.root / "assets"
        source.mkdir()
        for name in ["credit.json", "bus-factor.json", "top-contributor.json", "credit-bar.svg", ".env", "source.rs", "unrelated.svg"]:
            (source / name).write_text(name)
        (source / "authors").mkdir()
        (source / "authors/author-alice.svg").write_text("svg")
        (source / "authors/not-generated.svg").write_text("private")
        STAGE.stage(source, self.destination)
        paths = sorted(str(p.relative_to(self.destination)).replace("\\", "/") for p in self.destination.rglob("*") if p.is_file())
        self.assertEqual(paths, ["authors/author-alice.svg", "bus-factor.json", "credit-bar.svg", "credit.json", "top-contributor.json"])

    @unittest.skipUnless(shutil.which("bash"), "Bash is required for download resolution")
    def test_local_binary_resolution_and_invalid_download_inputs(self):
        """Resolve paths with spaces without network and reject unsupported releases."""
        executable = self.root / "local binary"
        executable.write_text("not executed")
        output = self.root / "outputs"
        env = os.environ.copy()
        env.update(GC_BINARY=str(executable), RUNNER_OS="Windows" if os.name == "nt" else "Linux",
                   RUNNER_ARCH="X64", GITHUB_OUTPUT=str(output), RUNNER_TEMP=str(self.root), GITHUB_ACTION_PATH=str(ROOT))
        result = subprocess.run(["bash", str(ROOT / "tools/action-download.sh")], cwd=ROOT, env=env, capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(output.read_text().strip(), "binary=" + str(executable.absolute()))
        env.update(GC_BINARY="", GC_VERSION="../not-a-tag")
        result = subprocess.run(["bash", str(ROOT / "tools/action-download.sh")], cwd=ROOT, env=env, capture_output=True, text=True, timeout=30)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("v-prefixed release tag", result.stderr)
        env.update(RUNNER_ARCH="unsupported")
        result = subprocess.run(["bash", str(ROOT / "tools/action-download.sh")], cwd=ROOT, env=env, capture_output=True, text=True, timeout=30)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Unsupported release runner", result.stderr)

    def test_packages_are_repeatable_and_round_trip_verification(self):
        """Archives contain the README/licenses and round-trip through the downloader."""
        previous = pathlib.Path.cwd()
        self.addCleanup(os.chdir, previous)
        os.chdir(self.root)
        for name in ("LICENSE-APACHE", "LICENSE-MIT", "README.md"):
            pathlib.Path(name).write_text(name)
        for target in ("x86_64-unknown-linux-gnu", "x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl", "x86_64-pc-windows-msvc"):
            executable = "git-credit.exe" if "windows" in target else "git-credit"
            binary = pathlib.Path("target") / target / "release" / executable
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"fixture executable")
            PACKAGE.package(target)
            archive = next(pathlib.Path("dist").glob("git-credit-" + target + ".*"))
            first = archive.read_bytes()
            PACKAGE.package(target)
            self.assertEqual(archive.read_bytes(), first)
            sums = self.root / "SHA256SUMS"
            sums.write_text(hashlib.sha256(first).hexdigest() + "  " + archive.name + "\n")
            output = self.destination / target
            output.mkdir()
            VERIFY.verify(archive, sums, output, executable)
            self.assertEqual((output / executable).read_bytes(), b"fixture executable")


@unittest.skipUnless(shutil.which("bash"), "Bash is required for composite publisher tests")
class Publisher(unittest.TestCase):
    """Exercise the actual shell control flow against a local Git command double."""

    def run_publisher(self, scenario):
        """Return command result and Git trace, with every Git operation mocked."""
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            commands = root / "commands"
            commands.mkdir()
            fake = commands / "git"
            fake.write_text("""#!/usr/bin/env bash
set -eu
printf '%s|' "$@" >> "$MOCK_LOG"
printf '\\n' >> "$MOCK_LOG"
if [ "${1:-}" = -C ]; then shift 2; fi
if [ "${1:-}" = -c ]; then shift 2; fi
if [ "${1:-}" = -c ]; then shift 2; fi
case "${1:-}" in
  rev-parse)
    case "$2" in
      --is-shallow-repository) if [ "$MOCK_SCENARIO" = shallow ]; then echo true; else echo false; fi ;;
      FETCH_HEAD) printf '%040d\\n' 2 ;;
      HEAD) printf '%040d\\n' 1 ;;
    esac ;;
  ls-remote)
    case "${*: -1}" in
      refs/heads/main) if [ "$MOCK_SCENARIO" = stale ]; then printf '%040d\\trefs/heads/main\\n' 3; else printf '%040d\\trefs/heads/main\\n' 1; fi ;;
      *) if [ "$MOCK_SCENARIO" = absent ]; then exit 2; fi; printf '%040d\\trefs/heads/badges\\n' 2 ;;
    esac ;;
  push) if [ "$MOCK_SCENARIO" = conflict ]; then echo 'mock lease rejected' >&2; exit 1; fi ;;
esac
""", encoding="utf-8", newline="\n")
            fake.chmod(0o755)
            assets = root / "assets"
            assets.mkdir()
            for name in ["credit.json", "bus-factor.json", "top-contributor.json"]:
                (assets / name).write_text("{}")
            log = root / "git.log"
            env = os.environ.copy()
            env.update(PATH=str(commands) + os.pathsep + env["PATH"], MOCK_LOG=str(log), MOCK_SCENARIO=scenario,
                       GC_OUT_DIR=str(assets), GC_PUBLISH_BRANCH="badges", GC_SOURCE_BRANCH="main", GC_DEFAULT_BRANCH="main",
                       GITHUB_REPOSITORY="fixture/repo", GITHUB_REF="refs/heads/main", GITHUB_SHA="0" * 39 + "1",
                       GITHUB_SERVER_URL="https://github.com", GITHUB_ACTION_PATH=str(ROOT), RUNNER_TEMP=str(root), RUNNER_OS="Windows" if os.name == "nt" else "Linux")
            result = subprocess.run(["bash", str(ROOT / "tools/action-publish.sh")], cwd=ROOT, env=env, capture_output=True, text=True, timeout=30)
            return result, log.read_text() if log.exists() else ""

    def test_existing_and_absent_refs_use_explicit_leases(self):
        """Use a fetched-SHA lease for existing refs and creation-only lease for new refs."""
        for scenario, previous in [("existing", "0" * 39 + "2"), ("absent", "")]:
            result, log = self.run_publisher(scenario)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("--force-with-lease=refs/heads/badges:" + previous + "|", log)
            self.assertIn("chore(badges): update for 000000000000", log)
            self.assertNotIn("--force|", log)

    def test_stale_and_shallow_runs_never_push(self):
        """Skip stale results and fail shallow sources before any publication."""
        for scenario, status in [("stale", 0), ("shallow", 1)]:
            result, log = self.run_publisher(scenario)
            self.assertEqual(result.returncode, status, result.stderr)
            self.assertNotIn("push|", log)
            self.assertIn("stale" if scenario == "stale" else "fetch-depth: 0", result.stderr)

    def test_failed_lease_is_not_retried_with_force(self):
        """A competing publisher must cause failure, never an unconditional overwrite."""
        result, log = self.run_publisher("conflict")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("mock lease rejected", result.stderr)
        self.assertEqual(log.count("push|"), 1)
        self.assertNotIn("--force|", log)


if __name__ == "__main__":
    unittest.main()
