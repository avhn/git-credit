//! Deterministic, bounded same-commit matching of complete inserted/removed runs.

use super::{Key, OwnedToken, Source, TokenChange, line_end, line_start, lines};
use similar::{Algorithm, DiffTag, capture_diff_slices};
use std::collections::{BTreeMap, BTreeSet};

/// Maximum source candidates per destination, selected by rare shared keys.
const CANDIDATES: usize = 32;
/// Bound diff work per candidate; larger blocks retain the exact-move fallback.
const MAX_TOKENS: usize = 4096;

/// Unmatched positions inside paired blocks, protected from a second move pass.
type Protected = BTreeMap<usize, Vec<usize>>;

/// One complete meaningful-line range in a staged destination.
struct Destination {
    change: usize,
    start: usize,
    end: usize,
}

/// An eligible match stores the exact ratio, not a rounded floating-point score.
struct Pair {
    source: usize,
    source_line: usize,
    destination: usize,
    matched: u64,
    total: u64,
    tokens: Vec<(usize, usize)>,
}

/// Trim incomplete edge lines, then accept at least three complete lines.
fn complete(tokens: &[OwnedToken], mut start: usize, mut end: usize) -> Option<(usize, usize)> {
    while start < end && !line_start(tokens, start) {
        start += 1;
    }
    while end > start && !line_end(tokens, end) {
        end -= 1;
    }
    let slice = tokens.get(start..end)?;
    (lines(slice) >= 3 && slice.len() <= MAX_TOKENS).then_some((start, end))
}

/// Collect contiguous editor candidates in path and physical-position order.
fn destinations(changes: &[(String, TokenChange)]) -> Vec<Destination> {
    let mut order: Vec<_> = (0..changes.len()).collect();
    order.sort_by_key(|index| changes.get(*index).map(|(path, _)| path));
    let mut result = Vec::new();
    for index in order {
        let Some((_, change)) = changes.get(index) else {
            continue;
        };
        let positions: Vec<_> = change.inserted.iter().copied().collect();
        for run in positions.chunk_by(|a, b| a.checked_add(1) == Some(*b)) {
            if let (Some(start), Some(end)) = (run.first(), run.last())
                && let Some((start, end)) = complete(&change.state.tokens, *start, end + 1)
            {
                result.push(Destination {
                    change: index,
                    start,
                    end,
                });
            }
        }
    }
    result
}

/// Match at most 32 sources per added block via its four rarest shared keys.
/// Best exact rational similarity wins, then source path/position and destination
/// path/position. Every source and destination block is consumed once. Unmatched
/// added tokens retain editor ownership and ordinary insertion labor.
pub(super) fn reconcile(
    changes: &mut [(String, TokenChange)],
    sources: &mut [Source],
) -> (usize, Protected) {
    let destinations = destinations(changes);
    let ranges: Vec<_> = sources
        .iter()
        .map(|source| complete(&source.tokens, 0, source.tokens.len()))
        .collect();
    let mut index: BTreeMap<Key, Vec<usize>> = BTreeMap::new();
    for (id, source) in sources.iter().enumerate() {
        let Some((start, end)) = ranges.get(id).copied().flatten() else {
            continue;
        };
        let keys: BTreeSet<_> = source
            .tokens
            .get(start..end)
            .unwrap_or_default()
            .iter()
            .map(|token| token.key.clone())
            .collect();
        for key in keys {
            index.entry(key).or_default().push(id);
        }
    }
    let mut pairs = Vec::new();
    for (id, destination) in destinations.iter().enumerate() {
        let Some((_, change)) = changes.get(destination.change) else {
            continue;
        };
        let after = change
            .state
            .tokens
            .get(destination.start..destination.end)
            .unwrap_or_default();
        let keys: BTreeSet<_> = after.iter().map(|token| &token.key).collect();
        let mut rare: Vec<_> = keys
            .into_iter()
            .filter_map(|key| index.get(key).map(|ids| (key, ids)))
            .collect();
        rare.sort_by(|a, b| a.1.len().cmp(&b.1.len()).then_with(|| a.0.cmp(b.0)));
        let mut candidates = BTreeSet::new();
        for (_, ids) in rare.into_iter().take(4) {
            for source in ids.iter().take(CANDIDATES) {
                if candidates.len() == CANDIDATES {
                    break;
                }
                candidates.insert(*source);
            }
        }
        for source in candidates {
            let Some((start, end)) = ranges.get(source).copied().flatten() else {
                continue;
            };
            let Some(before) = sources
                .get(source)
                .and_then(|source| source.tokens.get(start..end))
            else {
                continue;
            };
            if let Some(mut pair) = compare(before, after) {
                pair.source = source;
                pair.source_line = before.first().map_or(0, |token| token.line);
                pair.destination = id;
                pair.tokens.iter_mut().for_each(|(old, new)| {
                    *old += start;
                    *new += destination.start;
                });
                pairs.push(pair);
            }
        }
    }
    pairs.sort_by(|a, b| {
        (b.matched * a.total)
            .cmp(&(a.matched * b.total))
            .then_with(|| {
                sources
                    .get(a.source)
                    .map(|source| &source.path)
                    .cmp(&sources.get(b.source).map(|source| &source.path))
            })
            .then_with(|| a.source_line.cmp(&b.source_line))
            .then_with(|| a.destination.cmp(&b.destination))
            .then_with(|| a.source.cmp(&b.source))
    });
    apply_pairs(changes, sources, &destinations, pairs)
}

/// Apply disjoint winning matches without refunding unmatched editor tokens.
fn apply_pairs(
    changes: &mut [(String, TokenChange)],
    sources: &mut [Source],
    destinations: &[Destination],
    pairs: Vec<Pair>,
) -> (usize, Protected) {
    let mut protected = Protected::new();
    let mut used_sources = BTreeSet::new();
    let mut used_destinations = BTreeSet::new();
    let mut matched = 0;
    for pair in pairs {
        if used_sources.contains(&pair.source) || used_destinations.contains(&pair.destination) {
            continue;
        }
        let Some(destination) = destinations.get(pair.destination) else {
            continue;
        };
        let Some((_, change)) = changes.get_mut(destination.change) else {
            continue;
        };
        let Some(source) = sources.get_mut(pair.source) else {
            continue;
        };
        used_sources.insert(pair.source);
        used_destinations.insert(pair.destination);
        source.used.fill(true);
        for (old, new) in pair.tokens {
            if let (Some(old), Some(token)) =
                (source.tokens.get(old), change.state.tokens.get_mut(new))
            {
                token.owner = old.owner.clone();
                change.inserted.remove(&new);
                change.replaced.remove(&new);
                matched += 1;
            }
        }
        protected.entry(destination.change).or_default().extend(
            change
                .inserted
                .range(destination.start..destination.end)
                .copied(),
        );
    }
    (matched, protected)
}

/// Compute weighted token-diff similarity, accepting the inclusive 0.8 threshold.
fn compare(before: &[OwnedToken], after: &[OwnedToken]) -> Option<Pair> {
    let old: Vec<_> = before.iter().map(OwnedToken::key).collect();
    let new: Vec<_> = after.iter().map(OwnedToken::key).collect();
    let total = before
        .iter()
        .chain(after)
        .map(|token| u64::from(token.weight))
        .sum();
    let mut matched = 0;
    let mut tokens = Vec::new();
    for operation in capture_diff_slices(Algorithm::Myers, &old, &new) {
        if operation.tag() != DiffTag::Equal {
            continue;
        }
        for (old, new) in operation.old_range().zip(operation.new_range()) {
            if let (Some(left), Some(right)) = (before.get(old), after.get(new)) {
                matched += u64::from(left.weight.min(right.weight));
                tokens.push((old, new));
            }
        }
    }
    (matched * 10 >= total * 4).then_some(Pair {
        source: 0,
        source_line: 0,
        destination: 0,
        matched,
        total,
        tokens,
    })
}
