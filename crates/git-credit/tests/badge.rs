//! End-to-end badge rendering, report import and privacy fixtures.

use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;

/// Run fixture Git without hooks, signing or inherited global configuration.
fn git(path: &Path, args: &[&str], name: &str, email: &str) {
    let result = Command::new("git")
        .current_dir(path)
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.autocrlf=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", name)
        .env("GIT_AUTHOR_EMAIL", email)
        .env("GIT_COMMITTER_NAME", name)
        .env("GIT_COMMITTER_EMAIL", email)
        .env("GIT_AUTHOR_DATE", "2020-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2020-01-01T00:00:00Z")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

/// Build two authors with meaningful code and one excluded path.
fn fixture() -> TempDir {
    let repo = tempfile::tempdir().unwrap();
    git(repo.path(), &["init"], "Alice", "alice@example.test");
    fs::write(repo.path().join("a.rs"), "one\ntwo\n").unwrap();
    git(repo.path(), &["add", "."], "Alice", "alice@example.test");
    git(
        repo.path(),
        &["commit", "-m", "root"],
        "Alice",
        "alice@example.test",
    );
    fs::write(repo.path().join("b.rs"), "three\n").unwrap();
    git(
        repo.path(),
        &["add", "."],
        "Hidden Person",
        "hidden@example.test",
    );
    git(
        repo.path(),
        &["commit", "-m", "second"],
        "Hidden Person",
        "hidden@example.test",
    );
    repo
}

/// Invoke the built command with captured output and no terminal effects.
fn cli(repo: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_git-credit"))
        .current_dir(repo)
        .args(args)
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

/// Collect all files recursively for byte comparisons and privacy assertions.
fn files(path: &Path) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
    let mut output = std::collections::BTreeMap::new();
    for entry in fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        if entry.path().is_dir() {
            for (name, bytes) in files(&entry.path()) {
                output.insert(
                    std::path::PathBuf::from(entry.file_name()).join(name),
                    bytes,
                );
            }
        } else {
            output.insert(entry.file_name().into(), fs::read(entry.path()).unwrap());
        }
    }
    output
}

#[test]
fn analyzes_same_policy_and_round_trips_report_without_stdout() {
    let repo = fixture();
    fs::write(
        repo.path().join(".git-credit.toml"),
        "opt_out = [\"hidden@example.test\"]\nexclude = [\"b.rs\"]\n",
    )
    .unwrap();
    let analyzed = cli(repo.path(), &["analyze", "--json"]);
    assert!(analyzed.status.success());
    let output = tempfile::tempdir().unwrap();
    let result = cli(
        repo.path(),
        &["badge", "--out-dir", output.path().to_str().unwrap()],
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.stdout, [] as [u8; 0]);
    assert_eq!(result.stderr, [] as [u8; 0]);
    let expected: serde_json::Value = serde_json::from_slice(&analyzed.stdout).unwrap();
    let actual: serde_json::Value =
        serde_json::from_slice(&fs::read(output.path().join("credit.json")).unwrap()).unwrap();
    assert_eq!(expected, actual);
    let first = files(output.path());
    for bytes in first.values() {
        let text = String::from_utf8_lossy(bytes);
        assert!(!text.contains("Hidden Person"));
        assert!(!text.contains("hidden@example.test"));
    }
    for name in [
        "credit-bar.svg",
        "credit-card.svg",
        "bus-factor.svg",
        "bus-factor.json",
        "top-contributor.json",
        "credit.json",
        "authors/author-alice.svg",
    ] {
        assert!(output.path().join(name).is_file(), "missing {name}");
    }
    let imported = tempfile::tempdir().unwrap();
    let result = cli(
        imported.path(),
        &[
            "badge",
            "--from",
            output.path().join("credit.json").to_str().unwrap(),
            "--out-dir",
            imported.path().to_str().unwrap(),
            "-v",
        ],
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.stdout, [] as [u8; 0]);
    assert!(String::from_utf8_lossy(&result.stderr).contains("credit-bar.svg"));
    assert_eq!(first, files(imported.path()));
    let endpoint: serde_json::Value =
        serde_json::from_slice(&first[Path::new("top-contributor.json")]).unwrap();
    assert_eq!(endpoint["schemaVersion"], 1);
    assert_eq!(endpoint["message"], "Alice 100%");
}

#[test]
fn privacy_rerender_removes_old_identity_badges_and_preserves_unrelated_files() {
    let repo = fixture();
    let output = tempfile::tempdir().unwrap();
    let args = ["badge", "--out-dir", output.path().to_str().unwrap()];
    assert!(cli(repo.path(), &args).status.success());
    assert!(
        output
            .path()
            .join("authors/author-hidden-person.svg")
            .is_file()
    );
    fs::write(output.path().join("notes.txt"), "keep me").unwrap();
    fs::write(
        repo.path().join(".git-credit.toml"),
        "opt_out = [\"hidden@example.test\"]\n",
    )
    .unwrap();
    assert!(cli(repo.path(), &args).status.success());
    assert!(
        !output
            .path()
            .join("authors/author-hidden-person.svg")
            .exists()
    );
    assert!(output.path().join("authors/author-anonymous.svg").is_file());
    for bytes in files(output.path()).values() {
        let text = String::from_utf8_lossy(bytes);
        assert!(!text.contains("Hidden Person"));
        assert!(!text.contains("hidden@example.test"));
    }
    assert_eq!(
        fs::read_to_string(output.path().join("notes.txt")).unwrap(),
        "keep me"
    );
}

#[test]
fn selects_styles_and_rejects_bad_reports_before_writing() {
    let repo = fixture();
    let output = tempfile::tempdir().unwrap();
    let result = cli(
        repo.path(),
        &[
            "badge",
            "--out-dir",
            output.path().to_str().unwrap(),
            "--style",
            "card",
            "--theme",
            "dark",
            "--top",
            "1",
            "--width",
            "400",
        ],
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(files(output.path()).len(), 4);
    assert!(
        fs::read_to_string(output.path().join("credit-card.svg"))
            .unwrap()
            .contains("#161b22")
    );
    let report = output.path().join("credit.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&report).unwrap()).unwrap();
    for (key, bad) in [
        ("schema_version", serde_json::json!(2)),
        ("units", serde_json::json!("tokens")),
        ("head", serde_json::json!("bad")),
    ] {
        let original = value[key].clone();
        value[key] = bad;
        fs::write(&report, serde_json::to_vec(&value).unwrap()).unwrap();
        let target = output.path().join("not-created");
        let result = cli(
            repo.path(),
            &[
                "badge",
                "--from",
                report.to_str().unwrap(),
                "--out-dir",
                target.to_str().unwrap(),
            ],
        );
        assert_eq!(result.status.code(), Some(1));
        assert_eq!(result.stdout, [] as [u8; 0]);
        assert!(!target.exists());
        value[key] = original;
    }
    let bad = cli(
        repo.path(),
        &[
            "badge",
            "--out-dir",
            output.path().to_str().unwrap(),
            "--top",
            "0",
        ],
    );
    assert_eq!(bad.status.code(), Some(2));
}

#[cfg(unix)]
#[test]
fn refuses_symlinked_output() {
    let repo = fixture();
    let output = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let link = output.path().join("linked");
    std::os::unix::fs::symlink(outside.path(), &link).unwrap();
    let result = cli(repo.path(), &["badge", "--out-dir", link.to_str().unwrap()]);
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(files(outside.path()), std::collections::BTreeMap::new());
}

/// A symlinked directory inside the checkout must not redirect a relative output path.
#[cfg(unix)]
#[test]
fn refuses_relative_output_through_symlinked_component() {
    let repo = fixture();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), repo.path().join("docs")).unwrap();
    let result = cli(repo.path(), &["badge", "--out-dir", "docs/badges"]);
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(files(outside.path()), std::collections::BTreeMap::new());
}
