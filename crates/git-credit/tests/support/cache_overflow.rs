//! Black-box regressions for structurally valid, checksummed cache corruption.

use std::{collections::BTreeMap, fs};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{Fixture, cache_matches_cold};

/// Format-3 transport mirror: preserve every field while altering selected totals.
#[derive(Deserialize, Serialize)]
struct Pack {
    key: String,
    identity_key: String,
    allocation_key: String,
    ownership: BTreeMap<String, Summary>,
    groups: BTreeMap<u32, String>,
    head: String,
    commits: BTreeMap<String, Commit>,
    snapshots: BTreeMap<String, BTreeMap<String, String>>,
    files: BTreeMap<String, Vec<u8>>,
}

/// Cached ownership in integer millionths, including its conserved total.
#[derive(Deserialize, Serialize)]
struct Summary {
    total: u64,
    authors: BTreeMap<u32, u64>,
}

/// Commit accounting in the cache's declared field order.
#[derive(Deserialize, Serialize)]
struct Commit {
    identity: Vec<(u32, u32)>,
    labor: BTreeMap<u32, u64>,
    formatting: bool,
    bulk: bool,
    warnings: Vec<String>,
}

/// Rewrite a valid postcard pack with a fresh checksum, bypassing checksum-only
/// rejection so the semantic validation and resumed-run retry are exercised.
fn alter_labor(fixture: &Fixture, modify: impl FnOnce(&mut Pack)) {
    let path = fixture.dir.path().join(".git/git-credit/cache/replay.pack");
    let bytes = fs::read(&path).unwrap();
    let mut pack: Pack = postcard::from_bytes(&bytes[32..]).unwrap();
    modify(&mut pack);
    let payload = postcard::to_allocvec(&pack).unwrap();
    let mut bytes = Sha256::digest(&payload).to_vec();
    bytes.extend(payload);
    fs::write(path, bytes).unwrap();
}

#[test]
fn overflowing_cached_totals_are_rejected_and_rebuilt() {
    for token in [false, true] {
        let fixture = if token {
            Fixture::token()
        } else {
            Fixture::new()
        };
        fixture.write("a.rs", "original\n");
        fixture.commit("Alice", "alice@example.test", "root");
        fixture.commit("Alice", "alice@example.test", "empty");
        fixture.json(&[]);
        alter_labor(&fixture, |pack| {
            assert_eq!(pack.commits.len(), 2);
            for (record, amount) in pack.commits.values_mut().zip([u64::MAX, 1]) {
                record.labor = BTreeMap::from([(0, amount)]);
            }
        });
        cache_matches_cold(
            &fixture,
            &[],
            "Cache cold replay: fixed-point credit total overflow",
        );
        cache_matches_cold(&fixture, &[], "replayed 0 new commits");
    }
}

#[test]
fn overflow_after_new_work_retries_once_without_poisoning_the_cache() {
    for token in [false, true] {
        let fixture = if token {
            Fixture::token()
        } else {
            Fixture::new()
        };
        fixture.write("a.rs", "original\n");
        fixture.commit("Alice", "alice@example.test", "root");
        fixture.json(&[]);
        alter_labor(&fixture, |pack| {
            pack.commits.values_mut().next().unwrap().labor = BTreeMap::from([(0, u64::MAX)]);
        });
        fixture.write("b.rs", "new work\n");
        fixture.commit("Alice", "alice@example.test", "append");
        cache_matches_cold(
            &fixture,
            &[],
            "Cache resumed replay failed: fixed-point credit total overflow; retrying cold",
        );
        cache_matches_cold(&fixture, &[], "replayed 0 new commits");
    }
}

/// Semantic summary corruption must fall back even with a correct checksum.
#[test]
fn timeline_corrupt_or_missing_summaries_rebuild() {
    let fixture = Fixture::token();
    fixture.write("a.rs", "alpha beta\n");
    fixture.commit("Alice", "alice@example.test", "root");
    fixture.json(&["--timeline"]);
    for kind in 0..5 {
        alter_labor(&fixture, |pack| {
            let summary = pack.ownership.values_mut().next().unwrap();
            match kind {
                0 => summary.total += 1,
                1 => {
                    summary.authors.insert(u32::MAX, 0);
                }
                2 => {
                    summary.authors.insert(0, u64::MAX);
                }
                3 => pack.ownership.clear(),
                _ => pack.allocation_key.clear(),
            }
        });
        cache_matches_cold(&fixture, &["--timeline"], "Cache cold replay");
        cache_matches_cold(&fixture, &["--timeline"], "replayed 0 new commits");
    }
}

#[test]
fn privacy_aggregation_failure_retries_before_cache_publication() {
    let fixture = Fixture::token();
    fixture.write("a.rs", "original\n");
    fixture.commit("Alice", "alice@example.test", "root");
    fixture.write("b.rs", "other work\n");
    fixture.commit("Bob", "bob@example.test", "second");
    fixture.json(&[]);
    alter_labor(&fixture, |pack| {
        for record in pack.commits.values_mut() {
            let id = record.identity[0].0;
            record.labor = BTreeMap::from([(id, u64::MAX)]);
        }
    });
    fixture.write(
        ".git-credit.toml",
        "opt_out = [\"alice@example.test\", \"bob@example.test\"]\n",
    );
    cache_matches_cold(
        &fixture,
        &[],
        "Cache resumed replay failed: fixed-point credit total overflow; retrying cold",
    );
    cache_matches_cold(&fixture, &[], "replayed 0 new commits");
}
