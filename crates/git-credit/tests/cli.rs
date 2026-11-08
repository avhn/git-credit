//! Real Git fixtures with isolated configuration, fixed dates and exact totals.

use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;

/// Temporary repository whose commands cannot sign commits or execute hooks.
struct Fixture {
    dir: TempDir,
}
impl Fixture {
    /// Initialize a deterministic main branch without inherited user configuration.
    fn new() -> Self {
        let fixture = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        fixture.git(&["init"]);
        fixture.git(&["config", "core.autocrlf", "false"]);
        fixture
    }
    /// Run Git with isolated identities and fixed timestamps; fail with diagnostics.
    fn git(&self, args: &[&str]) -> Output {
        git(self.dir.path(), args, "Alice", "alice@example.test")
    }
    /// Create or replace a fixture file, creating its parent directories.
    fn write(&self, path: &str, bytes: impl AsRef<[u8]>) {
        let path = self.dir.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    /// Stage and commit the entire fixture tree under a fixed author.
    fn commit(&self, name: &str, email: &str, message: &str) {
        self.git(&["add", "-A"]);
        git(
            self.dir.path(),
            &["commit", "--allow-empty", "-m", message],
            name,
            email,
        );
    }
    /// Execute the built CLI with piped output and no color.
    fn cli(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_git-credit"))
            .arg("analyze")
            .arg(self.dir.path())
            .args(args)
            .env("NO_COLOR", "1")
            .output()
            .unwrap()
    }
    /// Parse a successful JSON report and expose unexpected stderr on failure.
    fn json(&self, args: &[&str]) -> Value {
        let mut flags = vec!["--json"];
        flags.extend_from_slice(args);
        let output = self.cli(&flags);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

/// Execute only local fixture Git operations with a known identity and configuration.
fn git(path: &Path, args: &[&str], name: &str, email: &str) -> Output {
    let output = Command::new("git")
        .current_dir(path)
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "init.defaultBranch=main",
            "-c",
            "core.autocrlf=false",
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.test",
        ])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", name)
        .env("GIT_AUTHOR_EMAIL", email)
        .env("GIT_COMMITTER_NAME", name)
        .env("GIT_COMMITTER_EMAIL", email)
        .env("GIT_AUTHOR_DATE", "2020-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2020-01-01T00:00:00Z")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
/// Locate a reported author by canonical email.
fn author<'a>(report: &'a Value, email: &str) -> &'a Value {
    report["authors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["email"] == email)
        .unwrap()
}

#[test]
fn raw_rename_diff_ignores_uncommitted_and_configured_attributes() {
    let fixture = Fixture::new();
    fixture.write("a.rs", "abcdefghij\nanchor one\nanchor two\nanchor three\n");
    fixture.commit("Alice", "alice@example.test", "root");
    fixture.git(&["mv", "a.rs", "b.rs"]);
    fixture.write("b.rs", "abcdefghiX\nanchor one\nanchor two\nanchor three\n");
    fixture.commit("Bob", "bob@example.test", "rename and edit");
    let baseline = fixture.json(&[]);
    assert_eq!(
        author(&baseline, "alice@example.test")["ownership_lines"],
        4.0
    );
    assert_eq!(author(&baseline, "bob@example.test")["labor"], 0.1);
    fixture.write(".gitattributes", "*.rs -diff\n");
    fixture.git(&["add", ".gitattributes"]);
    assert_eq!(fixture.json(&[]), baseline);
    fixture.write(".gitattributes", "*.rs diff=hostile\n");
    fixture.git(&["add", ".gitattributes"]);
    fixture.git(&[
        "config",
        "diff.hostile.textconv",
        "nonexistent-git-credit-test-command",
    ]);
    fixture.git(&[
        "config",
        "diff.hostile.command",
        "nonexistent-git-credit-test-command",
    ]);
    fixture.write(".git/info/attributes", "*.rs -diff\n");
    fixture.write("local-attributes", "*.rs -diff\n");
    fixture.git(&["config", "core.attributesFile", "local-attributes"]);
    assert_eq!(fixture.json(&[]), baseline);
}

#[test]
fn names_group_after_mailmap_with_frequent_email_and_late_opt_out() {
    let fixture = Fixture::new();
    fixture.write("src/a.rs", "one\n");
    fixture.commit("Alias", "old@example.test", "root");
    fixture.write("src/a.rs", "one\ntwo\n");
    fixture.commit(" ALICE ", "new@example.test", "second");
    fixture.write("src/a.rs", "one\ntwo\nthree\n");
    fixture.write(".mailmap", "Alice <old@example.test>\n");
    fixture.commit("alice", "new@example.test", "third");
    fixture.write(
        ".git-credit.toml",
        "exclude = ['.mailmap']\ngroup_by = 'name'\n",
    );
    let report = fixture.json(&[]);
    assert_eq!(report["policy"]["group_by"], "name");
    assert_eq!(report["authors"].as_array().unwrap().len(), 1);
    let alice = author(&report, "new@example.test");
    assert_eq!(alice["name"], "Alice");
    assert_eq!(
        alice["emails"],
        serde_json::json!(["new@example.test", "old@example.test"])
    );
    assert_eq!(alice["ownership_lines"], 3.0);
    assert_eq!(alice["labor"], 3.0);
    assert_eq!(
        report["directories"][0]["authors"][0]["email"],
        "new@example.test"
    );
    let separate = fixture.json(&["--by-email"]);
    assert_eq!(separate["authors"].as_array().unwrap().len(), 2);
    assert_eq!(separate["policy"]["group_by"], "email");
    assert_eq!(
        author(&separate, "old@example.test")["ownership_lines"],
        1.0
    );
    assert_eq!(
        author(&separate, "new@example.test")["ownership_lines"],
        2.0
    );
    fixture.write(
        ".git-credit.toml",
        "exclude = ['.mailmap']\ngroup_by = 'email'\n",
    );
    assert_eq!(fixture.json(&[]), separate);
    fixture.write(
        ".git-credit.toml",
        "exclude = ['.mailmap']\nopt_out = ['OLD@example.test']\n",
    );
    let private = fixture.json(&[]);
    assert_eq!(private["authors"].as_array().unwrap().len(), 1);
    assert_eq!(private["authors"][0]["name"], "anonymous");
    assert_eq!(private["authors"][0]["emails"], serde_json::json!([]));
    assert_eq!(private["authors"][0]["ownership_lines"], 3.0);
    assert_eq!(private["authors"][0]["labor"], 3.0);
    assert_eq!(private["directories"][0]["authors"][0]["email"], "");
    let serialized = serde_json::to_string(&private).unwrap();
    assert!(!serialized.contains("old@example.test"));
    assert!(!serialized.contains("new@example.test"));
    fixture.write(
        ".git-credit.toml",
        "exclude = ['.mailmap']\nopt_out = ['new@example.test']\n",
    );
    assert_eq!(fixture.json(&[]), private);
    fixture.write(".git-credit.toml", "group_by = 'invalid'\n");
    assert_eq!(fixture.cli(&[]).status.code(), Some(1));
    assert!(fixture.cli(&["--by-email"]).status.success());
}

#[test]
fn coauthor_aliases_count_once_and_frequency_ties_use_email_order() {
    let fixture = Fixture::new();
    fixture.write("a.rs", "one\ntwo\n");
    fixture.commit("Alice", "a@example.test", "root\n\nCo-authored-by: ALICE <z@example.test>\nCo-authored-by: alice <z@example.test>\nCo-authored-by: Bob <bob@example.test>");
    let report = fixture.json(&[]);
    assert_eq!(report["authors"].as_array().unwrap().len(), 2);
    assert_eq!(author(&report, "a@example.test")["ownership_lines"], 1.0);
    assert_eq!(author(&report, "bob@example.test")["ownership_lines"], 1.0);
    assert_eq!(
        author(&report, "a@example.test")["emails"],
        serde_json::json!(["a@example.test", "z@example.test"])
    );
}

#[test]
fn version_and_usage_errors() {
    let output = Command::new(env!("CARGO_BIN_EXE_git-credit"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!("git-credit {}", env!("CARGO_PKG_VERSION"))
    );
    let fixture = Fixture::new();
    for flags in [
        vec!["--keep", "NaN"],
        vec!["--keep", "1.1"],
        vec!["--exclude", "["],
        vec!["--depth", "bad"],
    ] {
        let output = fixture.cli(&flags);
        assert_eq!(output.status.code(), Some(2));
        assert_eq!(output.stdout, Vec::<u8>::new());
    }
    let output = fixture.cli(&[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("empty repository"));
}

#[test]
fn formatting_similarity_rewrite_deletion_and_rename() {
    let fixture = Fixture::new();
    fixture.write("a.rs", "abcdefghij\none\ntwo\nthree\n");
    fixture.commit("Alice", "alice@example.test", "root");
    fixture.write("a.rs", "  abcdefghij \r\n one\r\n two\r\n three\r\n\n");
    fixture.commit("Bob", "bob@example.test", "format");
    let report = fixture.json(&[]);
    assert_eq!(report["formatting_commits"], 1);
    assert_eq!(
        author(&report, "alice@example.test")["ownership_lines"],
        4.0
    );
    assert_eq!(author(&report, "alice@example.test")["labor"], 4.0);
    assert_eq!(report["authors"].as_array().unwrap().len(), 1);
    fixture.write("a.rs", "abcdefghiX\none\ntwo\nthree\n");
    fixture.commit("Bob", "bob@example.test", "minor");
    let report = fixture.json(&[]);
    assert_eq!(author(&report, "bob@example.test")["labor"], 0.1);
    assert_eq!(
        author(&report, "alice@example.test")["ownership_lines"],
        4.0
    );
    fixture.write("a.rs", "zzzzzzzzzz\none\ntwo\nthree\n");
    fixture.commit("Carol", "carol@example.test", "rewrite");
    let report = fixture.json(&[]);
    assert_eq!(
        author(&report, "carol@example.test")["ownership_lines"],
        1.0
    );
    fixture.write("a.rs", "one\ntwo\nthree\n");
    fixture.commit("Carol", "carol@example.test", "delete");
    fixture.git(&["mv", "a.rs", "renamed.rs"]);
    fixture.commit("Dave", "dave@example.test", "rename");
    let report = fixture.json(&[]);
    assert_eq!(report["commits_analyzed"], 6);
    assert_eq!(
        author(&report, "alice@example.test")["ownership_lines"],
        3.0
    );
    assert_eq!(author(&report, "alice@example.test")["labor"], 4.0);
    assert_eq!(
        author(&report, "carol@example.test")["ownership_lines"],
        0.0
    );
    assert_eq!(author(&report, "carol@example.test")["labor"], 1.0);
    assert_eq!(author(&report, "bob@example.test")["labor"], 0.1);
}

#[test]
fn cross_file_move_keeps_owners_and_zero_labor() {
    let fixture = Fixture::new();
    fixture.write("a.rs", "one\ntwo\nthree\nkeep a\nkeep b\nkeep c\nkeep d\n");
    fixture.write("b.rs", "existing a\nexisting b\nexisting c\nexisting d\n");
    fixture.commit("Alice", "alice@example.test", "root");
    fixture.write("a.rs", "keep a\nkeep b\nkeep c\nkeep d\n");
    fixture.write(
        "b.rs",
        "existing a\nexisting b\nexisting c\nexisting d\none\ntwo\nthree\n",
    );
    fixture.commit("Bob", "bob@example.test", "move block");
    let report = fixture.json(&[]);
    assert_eq!(
        author(&report, "alice@example.test")["ownership_lines"],
        11.0
    );
    assert_eq!(author(&report, "alice@example.test")["labor"], 11.0);
    assert_eq!(report["authors"].as_array().unwrap().len(), 1);
}

#[test]
fn head_mailmap_coauthors_and_opt_out_are_consistent() {
    let fixture = Fixture::new();
    fixture.write("src/a.rs", "one\ntwo\n");
    fixture.commit(
        "Alias",
        "alias@example.test",
        "root\n\nCo-AuThOrEd-By: Bob <bob@example.test>",
    );
    fixture.write(
        ".mailmap",
        "Alice <alice@example.test> Alias <alias@example.test>\n",
    );
    fixture.commit("Alice", "alice@example.test", "map");
    // Working tree edits must not replace the map at HEAD.
    fixture.write(
        ".mailmap",
        "Wrong <wrong@example.test> <alias@example.test>\n",
    );
    let report = fixture.json(&["--exclude", ".mailmap"]);
    assert_eq!(
        author(&report, "alice@example.test")["ownership_lines"],
        1.0
    );
    assert_eq!(author(&report, "bob@example.test")["labor"], 1.0);
    fixture.write(
        ".git-credit.toml",
        "opt_out = ['alice@example.test', 'bob@example.test']\nexclude = ['.mailmap']\n",
    );
    let report = fixture.json(&[]);
    assert_eq!(report["authors"].as_array().unwrap().len(), 1);
    assert_eq!(report["authors"][0]["name"], "anonymous");
    assert_eq!(report["authors"][0]["email"], "");
    assert_eq!(report["authors"][0]["ownership_lines"], 2.0);
    assert_eq!(report["authors"][0]["labor"], 2.0);
    assert_eq!(report["directories"][0]["authors"][0]["email"], "");
}

#[test]
fn exclusions_binary_size_and_json_contract() {
    let fixture = Fixture::new();
    fixture.write("src/deep/a.rs", "hello\n\nworld\n");
    for path in [
        "Cargo.lock",
        "nested/package-lock.json",
        "vendor/code.rs",
        "nested/node_modules/x.js",
        "dist/a.js",
        "nested/a.min.js",
        "a.min.css",
        "a.map",
    ] {
        fixture.write(path, "excluded");
    }
    fixture.write("binary.dat", b"hello\0world");
    fixture.write("large.txt", "x".repeat(100));
    fixture.commit("Alice", "alice@example.test", "root");
    fixture.write(".git-credit.toml", "max_file_size = 50\n");
    let report = fixture.json(&["--depth", "2"]);
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["algorithm_version"], 1);
    assert_eq!(report["head"].as_str().unwrap().len(), 40);
    assert_eq!(report["walk"], "first-parent");
    assert_eq!(report["units"], "lines");
    assert_eq!(report["policy"]["keep"], 0.7);
    assert_eq!(report["policy"]["max_file_size"], 50);
    assert_eq!(report["commits_analyzed"], 1);
    assert_eq!(
        author(&report, "alice@example.test")["ownership_lines"],
        2.0
    );
    assert_eq!(
        author(&report, "alice@example.test")["ownership_share"],
        1.0
    );
    assert_eq!(report["directories"][0]["path"], "src/deep");
    assert_eq!(report["directories"][0]["total_lines"], 2.0);
    assert_eq!(report["warnings"].as_array().unwrap().len(), 2);
    assert_eq!(
        fixture.json(&["--depth", "0"])["directories"],
        serde_json::json!([])
    );
    let output = fixture.cli(&["--no-color", "--quiet"]);
    assert!(output.status.success());
    assert_eq!(output.stderr, Vec::<u8>::new());
    assert!(!output.stdout.contains(&0x1b));
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("OWNERSHIP %"));
    assert!(text.contains("first-parent"));
}

#[test]
fn shallow_clone_fails_with_recovery_hint() {
    let source = Fixture::new();
    source.write("a.rs", "one");
    source.commit("Alice", "alice@example.test", "root");
    source.write("a.rs", "two");
    source.commit("Bob", "bob@example.test", "second");
    let target = tempfile::tempdir().unwrap();
    let source_path = source.dir.path().to_str().unwrap().replace('\\', "/");
    let url = if cfg!(windows) {
        format!("file:///{source_path}")
    } else {
        format!("file://{source_path}")
    };
    git(
        target.path(),
        &["clone", "--depth", "1", &url, "clone"],
        "Alice",
        "alice@example.test",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_git-credit"))
        .args(["analyze", "--json"])
        .arg(target.path().join("clone"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stdout, Vec::<u8>::new());
    assert!(String::from_utf8_lossy(&output.stderr).contains("git fetch --unshallow"));
}

#[test]
fn config_override_validation_and_indentation() {
    let fixture = Fixture::new();
    fixture.write("a.py", "abcdefghij");
    fixture.write("skip.rs", "unchanged");
    fixture.commit("Alice", "alice@example.test", "root");
    fixture.write("a.py", " abcdefghij");
    fixture.commit("Bob", "bob@example.test", "indentation");
    fixture.write(".git-credit.toml", "keep = 1.0\nexclude = ['skip.rs']\n");
    let report = fixture.json(&[]);
    assert_eq!(author(&report, "bob@example.test")["ownership_lines"], 1.0);
    let overridden = fixture.json(&["--keep", "0.7", "--exclude", "nothing"]);
    assert_eq!(
        author(&overridden, "alice@example.test")["ownership_lines"],
        2.0
    );
    assert_eq!(author(&overridden, "bob@example.test")["labor"], 0.04762);
    fixture.write(".git-credit.toml", "keep = nan\n");
    assert_eq!(fixture.cli(&[]).status.code(), Some(1));
    assert!(fixture.cli(&["--keep", "0.7"]).status.success());
    fixture.write(".git-credit.toml", "unknown_setting = true\n");
    assert_eq!(fixture.cli(&[]).status.code(), Some(1));
}

#[test]
fn fuzzy_rename_and_binary_transition_keep_historical_labor() {
    let fixture = Fixture::new();
    fixture.write("a.rs", "abcdefghij\nanchor one\nanchor two\nanchor three\n");
    fixture.commit("Alice", "alice@example.test", "root");
    fixture.git(&["mv", "a.rs", "b.rs"]);
    fixture.write("b.rs", "abcdefghiX\nanchor one\nanchor two\nanchor three\n");
    fixture.commit("Bob", "bob@example.test", "rename and edit");
    let report = fixture.json(&[]);
    assert_eq!(
        author(&report, "alice@example.test")["ownership_lines"],
        4.0
    );
    assert_eq!(author(&report, "bob@example.test")["labor"], 0.1);
    fixture.write("b.rs", b"binary\0content");
    fixture.commit("Bob", "bob@example.test", "become binary");
    let report = fixture.json(&[]);
    assert_eq!(
        author(&report, "alice@example.test")["ownership_lines"],
        0.0
    );
    assert_eq!(author(&report, "alice@example.test")["labor"], 4.0);
    assert_eq!(report["formatting_commits"], 0);
}

#[test]
fn configured_mailmap_and_duplicate_trailers_preserve_equal_shares() {
    let fixture = Fixture::new();
    fixture.write("a.rs", "one\ntwo\nthree");
    fixture.commit("Alias", "alias@example.test", "root\n\nCo-authored-by: Alias <alias@example.test>\nCo-authored-by: Bob <bob@example.test>\nCo-authored-by: Carol <carol@example.test>");
    fixture.write(
        "mapping",
        "Alice <alice@example.test> <alias@example.test>\n",
    );
    fixture.git(&["config", "mailmap.file", "mapping"]);
    fixture.write(
        ".git-credit.toml",
        "opt_out = ['bob@example.test', 'carol@example.test']\n",
    );
    let report = fixture.json(&[]);
    assert_eq!(
        author(&report, "alice@example.test")["ownership_lines"],
        1.000_002
    );
    assert_eq!(author(&report, "")["ownership_lines"], 1.999_998);
    assert_eq!(author(&report, "")["labor"], 1.999_998);
    let text = serde_json::to_string(&report).unwrap();
    assert!(!text.contains("bob@example.test"));
    assert!(!text.contains("carol@example.test"));
}

#[test]
fn bare_repositories_and_missing_parent_objects() {
    let fixture = Fixture::new();
    fixture.write("a.rs", "root");
    fixture.commit("Alice", "alice@example.test", "root");
    let root = String::from_utf8(fixture.git(&["rev-parse", "HEAD"]).stdout)
        .unwrap()
        .trim()
        .to_owned();
    fixture.write("a.rs", "next");
    fixture.commit("Bob", "bob@example.test", "next");
    let target = tempfile::tempdir().unwrap();
    git(
        target.path(),
        &[
            "clone",
            "--bare",
            fixture.dir.path().to_str().unwrap(),
            "bare",
        ],
        "Alice",
        "alice@example.test",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_git-credit"))
        .args(["analyze", "--json"])
        .arg(target.path().join("bare"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["commits_analyzed"], 2);
    // Only remove an object owned by this temporary fixture, never a real repo.
    fs::remove_file(
        fixture
            .dir
            .path()
            .join(".git/objects")
            .join(&root[..2])
            .join(&root[2..]),
    )
    .unwrap();
    let output = fixture.cli(&["--json"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stdout, Vec::<u8>::new());
    assert!(String::from_utf8_lossy(&output.stderr).contains("history objects"));
}

#[test]
fn first_parent_merge_counts_diff_once() {
    let fixture = Fixture::new();
    fixture.write("root.rs", "root");
    fixture.commit("Alice", "alice@example.test", "root");
    fixture.git(&["checkout", "-b", "branch"]);
    fixture.write("branch.rs", "branch");
    fixture.commit("Bob", "bob@example.test", "branch");
    fixture.git(&["checkout", "main"]);
    fixture.git(&["merge", "--no-ff", "branch", "-m", "merge"]);
    let report = fixture.json(&[]);
    assert_eq!(report["commits_analyzed"], 2);
    assert_eq!(
        author(&report, "alice@example.test")["ownership_lines"],
        2.0
    );
    assert_eq!(author(&report, "alice@example.test")["labor"], 2.0);
    assert_eq!(report["warnings"].as_array().unwrap().len(), 1);
}
