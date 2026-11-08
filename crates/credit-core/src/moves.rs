//! Exact same-commit block moves, with one-use source provenance.

use crate::{Change, LineOwner};

/// Restore ownership and cancel insertion labor for exact cross-file moves of at
/// least three consecutive normalized lines. Pass every changed or deleted file
/// from one commit (deletions are `apply_change` calls with empty new text).
///
/// Paths must uniquely identify files. Sources and destinations are considered
/// lexicographically by path, then by line position. Each removed line is consumed
/// at most once, so copies cannot multiply its credit. Repeated calls are safe:
/// matched destination lines cease to be candidates. Call before accumulating labor.
/// Returns the number of reassigned lines. Does not change `ChangeKind`: moves
/// remain meaningful changes, not formatting.
pub fn reconcile_moves(changes: &mut [(String, Change)]) -> usize {
    let mut order: Vec<_> = (0..changes.len()).collect();
    order.sort_by_key(|&i| changes.get(i).map(|(path, _)| path.clone()));
    let mut sources = Vec::new();
    for &index in &order {
        if let Some((path, change)) = changes.get(index) {
            for block in &change.removed {
                sources.push(Source {
                    path: path.clone(),
                    lines: block
                        .iter()
                        .map(|line| {
                            (
                                change.mode.normalize(&line.text).to_owned(),
                                line.owner.clone(),
                            )
                        })
                        .collect(),
                    used: vec![false; block.len()],
                });
            }
        }
    }
    let mut moved = 0;
    for index in order {
        if let Some((path, change)) = changes.get_mut(index) {
            for source in &mut sources {
                if source.path == *path {
                    continue;
                }
                moved += match_source(change, source);
            }
        }
    }
    // Consumed sources must also disappear so subsequent calls cannot reuse them.
    for (path, change) in changes {
        let mut remaining = Vec::new();
        for source in sources.iter().filter(|source| source.path == *path) {
            let mut block = Vec::new();
            for ((text, owner), used) in source.lines.iter().zip(&source.used) {
                if *used {
                    if !block.is_empty() {
                        remaining.push(std::mem::take(&mut block));
                    }
                } else {
                    block.push(crate::Line {
                        text: text.clone(),
                        owner: owner.clone(),
                    });
                }
            }
            if !block.is_empty() {
                remaining.push(block);
            }
        }
        change.removed = remaining;
    }
    moved
}

/// Removed normalized block plus consumption state shared across destinations.
struct Source {
    path: String,
    lines: Vec<(String, LineOwner)>,
    used: Vec<bool>,
}

/// Match maximal consecutive runs and subtract exactly their original labor shares.
fn match_source(change: &mut Change, source: &mut Source) -> usize {
    let mut moved = 0;
    for start in 0..change.state.lines.len() {
        if !change.added.contains_key(&start) {
            continue;
        }
        for old_start in 0..source.lines.len() {
            let length = (old_start..source.lines.len())
                .zip(start..change.state.lines.len())
                .take_while(|&(old, new)| {
                    source.used.get(old) == Some(&false)
                        && change.added.contains_key(&new)
                        && source
                            .lines
                            .get(old)
                            .zip(change.state.lines.get(new))
                            .is_some_and(|((text, _), line)| {
                                text == change.mode.normalize(&line.text)
                            })
                })
                .count();
            if length < 3 {
                continue;
            }
            for (old, new) in (old_start..old_start + length).zip(start..start + length) {
                if let Some((_, owner)) = source.lines.get(old)
                    && let Some(line) = change.state.lines.get_mut(new)
                {
                    line.owner = owner.clone();
                }
                if let Some(used) = source.used.get_mut(old) {
                    *used = true;
                }
                if let Some(delta) = change.added.remove(&new) {
                    for (id, amount) in delta {
                        if let Some(total) = change.labor.get_mut(&id) {
                            *total = total.saturating_sub(amount);
                        }
                    }
                }
            }
            moved += length;
            break;
        }
    }
    moved
}
