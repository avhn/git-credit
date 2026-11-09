//! Historical integer summaries, collected before final privacy remapping.

use std::collections::BTreeMap;

use credit_core::{AuthorId, LaborDelta, Ledger};

use crate::{Error, cache, git_error, history::History};

/// Optional independent activity and ownership series, in integer millionths.
#[derive(Debug, Default)]
pub struct Timeline {
    /// Weekly labor, sorted by UTC Monday start.
    pub activity: Vec<ActivityBucket>,
    /// Ownership along HEAD's first-parent ancestry, sorted by ancestry index.
    pub ownership: Vec<OwnershipSample>,
}

/// Meaningful labor in a UTC ISO week, including all reachable commits once.
#[derive(Debug)]
pub struct ActivityBucket {
    /// Monday 00:00 UTC, formatted as an RFC 3339 timestamp.
    pub week_start: String,
    /// Reachable commits in this week, including zero-labor commits.
    pub commits: u64,
    /// Total labor in millionths of a line.
    pub total: u64,
    /// Labor indexed by the report's privacy-filtered authors table.
    pub authors: LaborDelta,
}

/// Full-graph ownership at one first-parent commit, without historical file maps.
#[derive(Debug)]
pub struct OwnershipSample {
    /// Full commit object ID.
    pub commit: String,
    /// Zero-based position from the oldest first-parent ancestor, the chart x axis.
    pub first_parent_index: usize,
    /// Author time in UTC RFC 3339 form; metadata, not necessarily monotonic.
    pub author_timestamp: String,
    /// Total ownership in millionths of a line.
    pub total: u64,
    /// Ownership indexed by the report's privacy-filtered authors table.
    pub authors: LaborDelta,
}

/// Select at most 120 evenly spaced chain positions, always including both ends.
/// Root-anchored positions permit incremental reuse until the stride changes.
pub(crate) fn samples(history: &History, head: gix::ObjectId) -> BTreeMap<gix::ObjectId, usize> {
    let mut chain = Vec::new();
    let mut next = Some(head);
    while let Some(id) = next {
        chain.push(id);
        next = history
            .parents
            .get(&id)
            .and_then(|parents| parents.first())
            .copied();
    }
    chain.reverse();
    let last = chain.len().saturating_sub(1);
    let stride = last.div_ceil(119).max(1);
    chain
        .into_iter()
        .enumerate()
        .filter(|(index, _)| index % stride == 0 || *index == last)
        .map(|(index, id)| (id, index))
        .collect()
}

/// Read author time from immutable objects, never from cache metadata. Author
/// time reflects when work was written; committer time can change on rebase/merge.
fn author_time(repo: &gix::Repository, id: gix::ObjectId) -> Result<i64, Error> {
    let commit = repo.find_commit(id).map_err(git_error)?;
    let decoded = commit.decode().map_err(git_error)?;
    Ok(decoded
        .author()
        .map_err(git_error)?
        .time()
        .map_err(git_error)?
        .seconds)
}

/// Format UTC without lossy fallback for timestamps outside the supported range.
fn timestamp(seconds: i64) -> Result<String, Error> {
    gix::date::Time::new(seconds, 0)
        .format(gix::date::time::format::ISO8601_STRICT)
        .map_err(git_error)
}

/// Compute UTC Monday using Euclidean division, including dates before the epoch.
fn week_start(seconds: i64) -> Result<i64, Error> {
    let days = seconds.div_euclid(86_400);
    let monday = days - (days + 3).rem_euclid(7);
    monday
        .checked_mul(86_400)
        .ok_or_else(|| Error::InvalidInput("author week is outside timestamp range".into()))
}

/// Sum fixed-point amounts without silently wrapping a malformed cache total.
pub(crate) fn total<'a>(mut values: impl Iterator<Item = &'a u64>) -> Result<u64, Error> {
    values
        .try_fold(0_u64, |sum, amount| {
            sum.checked_add(*amount).ok_or(credit_core::Error::Overflow)
        })
        .map_err(Into::into)
}

/// Build activity from cached per-commit labor and selected ownership summaries.
/// The caller must apply the final identity mapping before exposing this result.
pub(crate) fn collect(
    repo: &gix::Repository,
    history: &History,
    pack: &cache::Pack,
    selected: &BTreeMap<gix::ObjectId, usize>,
) -> Result<Timeline, Error> {
    let mut weeks: BTreeMap<i64, (u64, Ledger)> = BTreeMap::new();
    for id in &history.order {
        let record = pack
            .commits
            .get(&id.to_string())
            .ok_or_else(|| Error::InvalidInput("missing timeline labor".into()))?;
        let (count, ledger) = weeks
            .entry(week_start(author_time(repo, *id)?)?)
            .or_default();
        *count = count.checked_add(1).ok_or(credit_core::Error::Overflow)?;
        ledger.add_labor(
            &record
                .labor
                .iter()
                .map(|(id, amount)| (AuthorId(*id), *amount))
                .collect(),
        )?;
    }
    let activity = weeks
        .into_iter()
        .map(|(week, (commits, ledger))| {
            Ok(ActivityBucket {
                week_start: timestamp(week)?,
                commits,
                total: total(ledger.labor.values())?,
                authors: ledger.labor,
            })
        })
        .collect::<Result<_, Error>>()?;
    let mut ownership = selected
        .iter()
        .map(|(id, index)| {
            let summary = pack
                .ownership
                .get(&id.to_string())
                .ok_or_else(|| Error::InvalidInput("missing timeline ownership".into()))?;
            Ok(OwnershipSample {
                commit: id.to_string(),
                first_parent_index: *index,
                author_timestamp: timestamp(author_time(repo, *id)?)?,
                total: summary.total,
                authors: summary
                    .authors
                    .iter()
                    .map(|(id, amount)| (AuthorId(*id), *amount))
                    .collect(),
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    ownership.sort_by_key(|sample| sample.first_parent_index);
    Ok(Timeline {
        activity,
        ownership,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sampling keeps endpoints and the cap across stride boundaries.
    #[test]
    fn timeline_sampling_is_bounded_and_root_anchored() {
        for length in [1, 2, 119, 120, 121, 238, 239, 240, 10_000] {
            let order: Vec<_> = (0..length)
                .map(|index| gix::ObjectId::from_hex(format!("{index:040x}").as_bytes()).unwrap())
                .collect();
            let parents = order
                .iter()
                .enumerate()
                .map(|(index, id)| {
                    (
                        *id,
                        index
                            .checked_sub(1)
                            .map(|index| vec![order[index]])
                            .unwrap_or_default(),
                    )
                })
                .collect();
            let history = History {
                order,
                parents,
                remaining_children: BTreeMap::new(),
            };
            let head = *history.order.last().unwrap();
            let selected = samples(&history, head);
            assert!(selected.len() <= 120);
            assert_eq!(selected.get(&history.order[0]), Some(&0));
            assert_eq!(selected.get(&head), Some(&(length - 1)));
        }
    }

    /// Epoch and pre-epoch weeks use Monday boundaries, not truncating division.
    #[test]
    fn timeline_week_boundaries_and_invalid_dates() {
        assert_eq!(week_start(0).unwrap(), -259_200);
        assert_eq!(week_start(-1).unwrap(), -259_200);
        assert_eq!(week_start(345_599).unwrap(), -259_200);
        assert_eq!(week_start(345_600).unwrap(), 345_600);
        assert!(week_start(i64::MIN).is_err());
        assert!(timestamp(i64::MAX).is_err());
        assert!(total([u64::MAX, 1].iter()).is_err());
    }
}
