//! Timeline conservation, privacy, topology and cache reconstruction fixtures.

use super::{Fixture, cache_matches_cold};
use serde_json::Value;

/// Extract a full object ID without assuming the fixture's object hash ordering.
fn head(fixture: &Fixture) -> String {
    String::from_utf8(fixture.git(&["rev-parse", "HEAD"]).stdout)
        .unwrap()
        .trim()
        .to_owned()
}

/// Amend only author time; the helper's fixed committer time remains unchanged.
fn date(fixture: &Fixture, value: &str) {
    fixture.git(&["commit", "--amend", "--no-edit", "--date", value]);
}

/// Check exact per-author integer conservation and report-local index references.
fn conserved(report: &Value) {
    let rows = report["authors"].as_array().unwrap();
    let mut labor = vec![0_u64; rows.len()];
    let mut commits = 0;
    for bucket in report["timeline"]["activity"].as_array().unwrap() {
        commits += bucket["commits"].as_u64().unwrap();
        let mut sum = 0;
        for amount in bucket["authors"].as_array().unwrap() {
            let index = usize::try_from(amount["author"].as_u64().unwrap()).unwrap();
            let value = amount["amount"].as_u64().unwrap();
            labor[index] += value;
            sum += value;
        }
        assert_eq!(sum, bucket["total"].as_u64().unwrap());
    }
    assert_eq!(commits, report["commits_analyzed"].as_u64().unwrap());
    for (row, amount) in rows.iter().zip(labor) {
        assert_eq!(
            row["labor"].as_f64().unwrap(),
            f64::from(u32::try_from(amount).unwrap()) / 1_000_000.0
        );
    }
    let samples = report["timeline"]["ownership"].as_array().unwrap();
    let last = samples.last().unwrap();
    assert_eq!(last["commit"], report["head"]);
    for sample in samples {
        let sum: u64 = sample["authors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|amount| {
                assert!(amount["author"].as_u64().unwrap() < u64::try_from(rows.len()).unwrap());
                amount["amount"].as_u64().unwrap()
            })
            .sum();
        assert_eq!(sum, sample["total"].as_u64().unwrap());
    }
    let mut owned = vec![0_u64; rows.len()];
    for amount in last["authors"].as_array().unwrap() {
        owned[usize::try_from(amount["author"].as_u64().unwrap()).unwrap()] =
            amount["amount"].as_u64().unwrap();
    }
    for (row, amount) in rows.iter().zip(owned) {
        assert_eq!(
            row["ownership_lines"].as_f64().unwrap(),
            f64::from(u32::try_from(amount).unwrap()) / 1_000_000.0
        );
    }
}

/// Author dates cross a UTC Monday boundary, independently of committer dates.
#[test]
fn weekly_activity_and_merge_samples_conserve_credit() {
    let fixture = Fixture::token();
    fixture.write("a.rs", "alpha beta gamma\n");
    fixture.commit("Alice", "alice@example.test", "root");
    date(&fixture, "2020-01-05T23:59:59Z");
    let root = head(&fixture);
    fixture.git(&["branch", "topic"]);
    fixture.write("b.rs", "main work\n");
    fixture.commit("Bob", "bob@example.test", "main");
    date(&fixture, "2020-01-06T02:00:00+02:00");
    let main = head(&fixture);
    fixture.git(&["checkout", "topic"]);
    fixture.write("c.rs", "branch work\n");
    fixture.commit("Carol", "carol@example.test", "topic");
    date(&fixture, "2020-01-07T00:00:00Z");
    let branch = head(&fixture);
    fixture.git(&["checkout", "main"]);
    fixture.git(&["merge", "--no-ff", "topic", "-m", "merge"]);
    let report = fixture.json(&["--timeline"]);
    conserved(&report);
    let weeks = report["timeline"]["activity"].as_array().unwrap();
    assert_eq!(weeks.len(), 2);
    assert_eq!(weeks[0]["week_start"], "2019-12-30T00:00:00+00:00");
    assert_eq!(weeks[1]["week_start"], "2020-01-06T00:00:00+00:00");
    assert_eq!(weeks[0]["commits"], 2);
    assert_eq!(weeks[1]["commits"], 2);
    let samples = report["timeline"]["ownership"].as_array().unwrap();
    assert_eq!(samples.len(), 3);
    assert_eq!(samples[0]["commit"], root);
    assert_eq!(samples[1]["commit"], main);
    assert!(samples.iter().all(|sample| sample["commit"] != branch));
    assert_eq!(samples[2]["first_parent_index"], 2);
    // The merge's author date goes backwards; ancestry remains the x axis.
    assert!(samples[2]["author_timestamp"].as_str() < samples[1]["author_timestamp"].as_str());
    cache_matches_cold(&fixture, &["--timeline"], "replayed 0 new commits");
}

/// Late aliases anonymize even authors whose content no longer survives at HEAD.
#[test]
fn late_alias_opt_out_applies_to_every_historical_point() {
    let fixture = Fixture::token();
    fixture.write("a.rs", "private work\n");
    fixture.commit("Private", "old@example.test", "root");
    fixture.json(&["--timeline"]);
    fixture.write("a.rs", "");
    fixture.commit("Other", "other@example.test", "delete");
    fixture.commit("Private", "hidden@example.test", "late alias");
    fixture.write(".git-credit.toml", "opt_out = ['hidden@example.test']\n");
    let output = fixture.cli(&["--timeline", "--json"]);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(!text.contains("old@example.test"));
    assert!(!text.contains("hidden@example.test"));
    let report: Value = serde_json::from_str(&text).unwrap();
    conserved(&report);
    assert_eq!(report["authors"].as_array().unwrap().len(), 1);
    assert_eq!(report["authors"][0]["name"], "anonymous");
    assert_eq!(
        report["timeline"]["ownership"][0]["authors"][0]["author"],
        0
    );
    cache_matches_cold(&fixture, &["--timeline"], "replayed 0 new commits");
}

/// Missing historical samples reconstruct cold; subsequent appends resume exactly.
#[test]
fn optional_timeline_reconstructs_and_resumes_without_changing_baseline() {
    let fixture = Fixture::token();
    fixture.write("a.rs", "alpha beta gamma\n");
    fixture.commit("Alice", "alice@example.test", "root");
    let baseline = fixture.cli(&["--json"]);
    let table = fixture.cli(&[]);
    assert!(fixture.json(&[]).get("timeline").is_none());
    cache_matches_cold(
        &fixture,
        &["--timeline"],
        "missing timeline sample coverage",
    );
    assert_eq!(baseline.stdout, fixture.cli(&["--json"]).stdout);
    assert_eq!(table.stdout, fixture.cli(&[]).stdout);
    fixture.write("a.rs", "alpha changed gamma\n");
    fixture.commit("Bob", "bob@example.test", "edit");
    cache_matches_cold(&fixture, &["--timeline"], "replayed 1 new commits");
    conserved(&fixture.json(&["--timeline"]));
    fixture.write(".git-credit.toml", "timeline = true\n");
    assert_eq!(fixture.json(&[]), fixture.json(&["--timeline"]));
    assert!(!fixture.cli(&["--first-parent"]).status.success());
}

/// A newly merged identity preceding existing allocation invalidates rounded data.
#[test]
fn identity_reordering_merge_rebuilds_summaries() {
    let fixture = Fixture::token();
    fixture.write("a.rs", "alpha beta gamma\n");
    fixture.commit("Alice", "alice@example.test", "root");
    fixture.git(&["branch", "topic"]);
    fixture.write("a.rs", "alpha changed gamma\n");
    fixture.commit("Bob", "bob@example.test", "main");
    let main = head(&fixture);
    fixture.json(&["--timeline"]);
    fixture.git(&["checkout", "topic"]);
    fixture.write("branch.rs", "branch work\n");
    fixture.commit("Carol", "carol@example.test", "branch");
    // Vary only the message until the ready-set SHA ordering puts Carol first.
    let mut before = head(&fixture) < main;
    for nonce in 0..1000 {
        if before {
            break;
        }
        fixture.git(&["commit", "--amend", "-m", &format!("branch {nonce}")]);
        before = head(&fixture) < main;
    }
    assert!(before);
    fixture.git(&["checkout", "main"]);
    fixture.git(&["merge", "--no-ff", "topic", "-m", "merge"]);
    cache_matches_cold(
        &fixture,
        &["--timeline"],
        "timeline identity allocation changed",
    );
    conserved(&fixture.json(&["--timeline"]));
}
