//! Parent provenance and one-use canonical token block moves.

use super::{Key, OwnedToken, TokenChange, TokenState};
use std::collections::BTreeMap;

/// Adopt a caller-verified exact parent blob, clearing editor work but preserving
/// first-parent removals as move sources. Call before bulk reconciliation.
pub fn adopt_token_parent(change: &mut TokenChange, parent: &TokenState) {
    change.state = parent.clone();
    change.inserted.clear();
    change.replacements.clear();
    change.replaced.clear();
    change.bulk_mappings.clear();
}

/// Restore inserted tokens found in other parents, in supplied parent order and
/// then earliest token position. Repeated text uses the first matching owner,
/// matching the line engine's provenance rule rather than consuming parent tokens.
/// Equal first-parent tokens are never overwritten. Labor is computed from the
/// remaining insertion set, so refunds cannot round twice. Returns restored tokens.
/// Call before moves and bulk detection.
pub fn reconcile_token_parents<'a>(
    change: &mut TokenChange,
    parents: impl IntoIterator<Item = &'a TokenState>,
) -> usize {
    if change.inserted.is_empty() {
        return 0;
    }
    let mut owners = BTreeMap::new();
    for parent in parents {
        for token in &parent.tokens {
            owners.entry(token.key()).or_insert(&token.owner);
        }
    }
    let mut matched = Vec::new();
    for &position in &change.inserted {
        if let Some(token) = change.state.tokens.get_mut(position)
            && let Some(owner) = owners.get(&token.key())
        {
            token.owner = (*owner).clone();
            matched.push(position);
        }
    }
    for position in &matched {
        change.inserted.remove(position);
        change.replaced.remove(position);
    }
    matched.len()
}

/// Removed provenance with a one-use bit for every token.
struct Source {
    path: String,
    tokens: Vec<OwnedToken>,
    used: Vec<bool>,
}

/// Collect a conservative union of removed token occurrences across parents.
/// Shared ancestry cannot provide duplicate source capacity for copied blocks.
fn sources(
    changes: &[(String, TokenChange)],
    seen: &mut BTreeMap<(String, Key), usize>,
) -> Vec<Source> {
    let mut result = Vec::new();
    let mut counts = BTreeMap::new();
    let mut ordered: Vec<_> = changes.iter().collect();
    ordered.sort_by(|a, b| a.0.cmp(&b.0));
    for (path, change) in ordered {
        for run in &change.removed {
            let mut tokens = Vec::new();
            for token in run {
                let key = (path.clone(), token.key.clone());
                let count = counts.entry(key.clone()).or_insert(0);
                *count += 1;
                if *count > seen.get(&key).copied().unwrap_or(0) {
                    tokens.push(token.clone());
                } else {
                    push_source(&mut result, path, &mut tokens);
                }
            }
            push_source(&mut result, path, &mut tokens);
        }
    }
    for (key, count) in counts {
        let previous = seen.entry(key).or_default();
        *previous = (*previous).max(count);
    }
    result
}

/// Flush one nonempty run without copying its token payload.
fn push_source(result: &mut Vec<Source>, path: &str, tokens: &mut Vec<OwnedToken>) {
    if !tokens.is_empty() {
        result.push(Source {
            path: path.to_owned(),
            used: vec![false; tokens.len()],
            tokens: std::mem::take(tokens),
        });
    }
}

/// Restore cross-file canonical runs spanning at least three meaningful source
/// lines and three meaningful destination lines. Whitespace and physical blank
/// lines do not participate. Matching starts and ends at meaningful line bounds.
/// Paths and positions determine ties; sources are consumed once. Other-parent
/// changes supply removals only, never labor or output state. Repeated calls are
/// safe because consumed first-parent sources and destination candidates disappear.
/// Returns the number of restored tokens, not a line-equivalent credit amount.
pub fn reconcile_token_moves(
    changes: &mut [(String, TokenChange)],
    parents: &[Vec<(String, TokenChange)>],
) -> usize {
    if changes.iter().all(|(_, change)| change.moves_reconciled) {
        return 0;
    }
    let mut seen = BTreeMap::new();
    let mut available = sources(changes, &mut seen);
    for parent in parents {
        available.extend(sources(parent, &mut seen));
    }
    let mut order: Vec<_> = (0..changes.len()).collect();
    order.sort_by_key(|&i| changes.get(i).map(|(path, _)| path.clone()));
    let mut matched = 0;
    for index in order {
        if let Some((path, change)) = changes.get_mut(index) {
            for source in &mut available {
                if source.path != *path {
                    matched += match_source(change, source);
                }
            }
        }
    }
    // Preserve only unconsumed provenance in staged first-parent changes.
    for (path, change) in changes {
        change.moves_reconciled = true;
        change.removed.clear();
        for source in available.iter().filter(|source| source.path == *path) {
            let mut run = Vec::new();
            for (token, used) in source.tokens.iter().zip(&source.used) {
                if *used {
                    if !run.is_empty() {
                        change.removed.push(std::mem::take(&mut run));
                    }
                } else {
                    run.push(token.clone());
                }
            }
            if !run.is_empty() {
                change.removed.push(run);
            }
        }
    }
    matched
}

/// Whether a position is the start of a meaningful line in a token run.
fn line_start(tokens: &[OwnedToken], index: usize) -> bool {
    tokens.get(index).is_some_and(|token| token.first_in_line)
}

/// Full physical line boundaries survive fragmented diff and parent-union runs.
fn line_end(tokens: &[OwnedToken], index: usize) -> bool {
    index > 0
        && tokens
            .get(index - 1)
            .is_some_and(|token| token.last_in_line)
}

/// Count meaningful lines in a nonempty contiguous token slice.
fn lines(tokens: &[OwnedToken]) -> usize {
    tokens.chunk_by(|a, b| a.line == b.line).count()
}

/// Index canonical three-token prefixes, then extend matching unused runs.
fn match_source(change: &mut TokenChange, source: &mut Source) -> usize {
    if lines(&source.tokens) < 3 || change.inserted.len() < 3 {
        return 0;
    }
    let mut starts = BTreeMap::new();
    for (position, triple) in source.tokens.windows(3).enumerate() {
        if line_start(&source.tokens, position) {
            let key: Vec<_> = triple.iter().map(|token| token.key.clone()).collect();
            starts.entry(key).or_insert_with(Vec::new).push(position);
        }
    }
    let mut matched = 0;
    for destination in 0..change.state.tokens.len() {
        if !change.inserted.contains(&destination) || !line_start(&change.state.tokens, destination)
        {
            continue;
        }
        let key: Vec<_> = change
            .state
            .tokens
            .iter()
            .skip(destination)
            .take(3)
            .map(|token| token.key.clone())
            .collect();
        let Some(candidates) = starts.get(&key) else {
            continue;
        };
        for &start in candidates {
            let mut length = source
                .tokens
                .iter()
                .skip(start)
                .zip(change.state.tokens.iter().skip(destination))
                .enumerate()
                .take_while(|(offset, (old, new))| {
                    source.used.get(start + offset) == Some(&false)
                        && change.inserted.contains(&(destination + offset))
                        && old.key == new.key
                })
                .count();
            // A maximal token run may finish partway through a changed line.
            while length > 0
                && (!line_end(&source.tokens, start + length)
                    || !line_end(&change.state.tokens, destination + length))
            {
                length -= 1;
            }
            let before = source.tokens.get(start..start + length).unwrap_or_default();
            let after = change
                .state
                .tokens
                .get(destination..destination + length)
                .unwrap_or_default();
            if lines(before) < 3 || lines(after) < 3 {
                continue;
            }
            for (offset, old) in before.iter().enumerate() {
                if let Some(new) = change.state.tokens.get_mut(destination + offset) {
                    new.owner = old.owner.clone();
                }
                if let Some(used) = source.used.get_mut(start + offset) {
                    *used = true;
                }
                change.inserted.remove(&(destination + offset));
                change.replaced.remove(&(destination + offset));
            }
            matched += length;
            break;
        }
    }
    matched
}
