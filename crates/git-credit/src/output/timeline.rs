//! Serialize exact historical totals with report-local, sorted author references.

use anyhow::Context;
use credit_core::{AuthorId, LaborDelta};
use credit_git::Report;
use serde::Serialize;

use super::AuthorRow;

/// Integer series carry explicit units separate from the report's line display.
#[derive(Serialize)]
pub(super) struct Timeline<'a> {
    units: &'static str,
    activity: Vec<Activity<'a>>,
    ownership: Vec<Ownership<'a>>,
}

/// One reference to the serialized report authors array, including anonymous.
#[derive(Serialize)]
struct Amount {
    author: usize,
    amount: u64,
}

/// Weekly additive labor; zero-labor commits still contribute to the count.
#[derive(Serialize)]
struct Activity<'a> {
    week_start: &'a str,
    commits: u64,
    total: u64,
    authors: Vec<Amount>,
}

/// Sample metadata is separate from the ordinal x axis.
#[derive(Serialize)]
struct Ownership<'a> {
    commit: &'a str,
    first_parent_index: usize,
    author_timestamp: &'a str,
    total: u64,
    authors: Vec<Amount>,
}

/// Keep historical-only contributors available for index references in JSON.
pub(super) fn historical(report: &Report, id: AuthorId) -> bool {
    report.timeline.as_ref().is_some_and(|timeline| {
        timeline
            .ownership
            .iter()
            .any(|sample| sample.authors.get(&id).is_some_and(|amount| *amount > 0))
    })
}

/// Translate final internal IDs into serialized row positions, failing closed if
/// any positive historical attribution has no privacy-filtered output identity.
pub(super) fn build<'a>(
    report: &'a Report,
    rows: &[AuthorRow<'_>],
) -> anyhow::Result<Option<Timeline<'a>>> {
    let Some(timeline) = &report.timeline else {
        return Ok(None);
    };
    // IDs index the report table densely; build once rather than searching names
    // for every author. Missing entries represent deliberately omitted zero rows.
    let mut mapping = vec![None; report.authors.len()];
    for (index, row) in rows.iter().enumerate() {
        *mapping
            .get_mut(row.id.0 as usize)
            .context("timeline author row outside identity table")? = Some(index);
    }
    let amounts = |totals: &LaborDelta| -> anyhow::Result<Vec<Amount>> {
        let mut result = totals
            .iter()
            .filter(|(_, amount)| **amount > 0)
            .map(|(id, amount)| {
                Ok(Amount {
                    author: mapping
                        .get(id.0 as usize)
                        .copied()
                        .flatten()
                        .context("missing timeline author row")?,
                    amount: *amount,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        result.sort_by_key(|amount| amount.author);
        Ok(result)
    };
    Ok(Some(Timeline {
        units: "millionths_of_line",
        activity: timeline
            .activity
            .iter()
            .map(|bucket| {
                Ok(Activity {
                    week_start: &bucket.week_start,
                    commits: bucket.commits,
                    total: bucket.total,
                    authors: amounts(&bucket.authors)?,
                })
            })
            .collect::<anyhow::Result<_>>()?,
        ownership: timeline
            .ownership
            .iter()
            .map(|sample| {
                Ok(Ownership {
                    commit: &sample.commit,
                    first_parent_index: sample.first_parent_index,
                    author_timestamp: &sample.author_timestamp,
                    total: sample.total,
                    authors: amounts(&sample.authors)?,
                })
            })
            .collect::<anyhow::Result<_>>()?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use credit_core::Ledger;
    use credit_git::{ActivityBucket, Author, Granularity, OwnershipSample, Walk};

    /// Large reverse-sorted identity tables exercise the formerly quadratic path
    /// without a flaky wall-clock assertion. Internal IDs must never enter JSON.
    #[test]
    fn fifty_thousand_authors_map_by_id_after_sorting() {
        let count = 50_000_u32;
        let totals: LaborDelta = (0..count)
            .map(|id| (AuthorId(id), u64::from(id) + 1))
            .collect();
        let total = totals.values().sum();
        let report = Report {
            timeline: Some(credit_git::Timeline {
                activity: vec![ActivityBucket {
                    week_start: "2020-01-06T00:00:00+00:00".into(),
                    commits: 1,
                    total,
                    authors: totals.clone(),
                }],
                ownership: vec![OwnershipSample {
                    commit: "a".repeat(40),
                    first_parent_index: 0,
                    author_timestamp: "2020-01-06T00:00:00+00:00".into(),
                    total,
                    authors: totals.clone(),
                }],
            }),
            cache_diagnostics: Vec::new(),
            granularity: Granularity::Token,
            bulk_edit_commits: 0,
            walk: Walk::Full,
            peak_retained_states: 0,
            head: "a".repeat(40),
            commits_analyzed: 1,
            formatting_commits: 0,
            authors: (0..count)
                .map(|id| Author {
                    name: "Contributor".into(),
                    email: format!("{id}@example.test"),
                    emails: Vec::new(),
                })
                .collect(),
            ledger: Ledger {
                ownership: totals.clone(),
                labor: totals,
            },
            directories: Vec::new(),
            warnings: Vec::new(),
        };
        let mut rows = super::super::rows(&report);
        assert_eq!(rows[0].id, AuthorId(count - 1));
        assert!(serde_json::to_value(&rows[0]).unwrap().get("id").is_none());
        let timeline = build(&report, &rows).unwrap().unwrap();
        for amounts in [
            &timeline.activity[0].authors,
            &timeline.ownership[0].authors,
        ] {
            assert_eq!(amounts.len(), count as usize);
            for (index, amount) in amounts.iter().enumerate() {
                assert_eq!(amount.author, index);
                assert_eq!(
                    amount.amount,
                    u64::from(count) - u64::try_from(index).unwrap()
                );
            }
        }
        rows.pop();
        assert!(build(&report, &rows).is_err());
        rows[0].id = AuthorId(count);
        assert!(build(&report, &rows).is_err());
    }
}
