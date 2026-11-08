//! End-to-end checks of the compiled binary's command-line surface.

use std::process::Command;

fn git_credit() -> Command {
    Command::new(env!("CARGO_BIN_EXE_git-credit"))
}

#[test]
fn version_flag_prints_package_version() {
    let output = git_credit().arg("--version").output().expect("binary runs");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.trim(),
        format!("git-credit {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn unimplemented_analyze_fails_on_stderr_only() {
    let output = git_credit()
        .args(["analyze", "."])
        .output()
        .expect("binary runs");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty(), "stdout must stay pipe-clean");
    assert!(String::from_utf8_lossy(&output.stderr).contains("not implemented"));
}
