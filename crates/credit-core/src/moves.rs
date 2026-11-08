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
    let mut seen = std::collections::BTreeMap::new();
    let sources = move_sources(changes, &mut seen);
    reconcile_sources(changes, &order, sources)
}

/// Reconcile cross-file moves using removals from every merge parent. Additional
/// parent diffs are source-only: their output states and labor are ignored.
/// Call once per merge, before accumulating labor. First-parent sources take
/// precedence, followed by parents in supplied order. Overlapping normalized
/// text occurrences at the same path are counted once across parents, preventing
/// shared ancestry from funding copies. This conservative union can fragment
/// overlapping blocks; the ordinary three-consecutive-line minimum still applies.
pub fn reconcile_merge_moves(
    changes: &mut [(String, Change)],
    parents: &[Vec<(String, Change)>],
) -> usize {
    let mut seen = std::collections::BTreeMap::new();
    let mut sources = move_sources(changes, &mut seen);
    for parent in parents {
        sources.extend(move_sources(parent, &mut seen));
    }
    let mut order: Vec<_> = (0..changes.len()).collect();
    order.sort_by_key(|&i| changes.get(i).map(|(path, _)| path.clone()));
    reconcile_sources(changes, &order, sources)
}

/// Union removed occurrences conservatively, splitting blocks at prior sources.
fn move_sources(
    changes: &[(String, Change)],
    seen: &mut std::collections::BTreeMap<(String, String), usize>,
) -> Vec<Source> {
    let mut sources = Vec::new();
    let mut counts = std::collections::BTreeMap::new();
    let mut order: Vec<_> = changes.iter().collect();
    order.sort_by(|a, b| a.0.cmp(&b.0));
    for (path, change) in order {
        for block in &change.removed {
            let mut lines = Vec::new();
            for line in block {
                let text = change.mode.normalize(&line.text).to_owned();
                let key = (path.clone(), text.clone());
                let occurrence = counts.entry(key.clone()).or_insert(0);
                *occurrence += 1;
                if *occurrence > seen.get(&key).copied().unwrap_or(0) {
                    lines.push((text, line.owner.clone()));
                } else if !lines.is_empty() {
                    sources.push(Source {
                        path: path.clone(),
                        used: vec![false; lines.len()],
                        lines: std::mem::take(&mut lines),
                    });
                }
            }
            if !lines.is_empty() {
                sources.push(Source {
                    path: path.clone(),
                    used: vec![false; lines.len()],
                    lines,
                });
            }
        }
    }
    for (key, count) in counts {
        let previous = seen.entry(key).or_default();
        *previous = (*previous).max(count);
    }
    sources
}

/// Match staged destinations and retain only unused removal provenance.
fn reconcile_sources(
    changes: &mut [(String, Change)],
    order: &[usize],
    mut sources: Vec<Source>,
) -> usize {
    let mut moved = 0;
    for &index in order {
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
    if source.lines.len() < 3 || change.added.len() < 3 {
        return 0;
    }
    let mut starts = std::collections::BTreeMap::new();
    for (position, triple) in source.lines.windows(3).enumerate() {
        if let [a, b, c] = triple {
            starts
                .entry((a.0.as_str(), b.0.as_str(), c.0.as_str()))
                .or_insert_with(Vec::new)
                .push(position);
        }
    }
    let mut moved = 0;
    for start in 0..change.state.lines.len() {
        if !change.added.contains_key(&start) {
            continue;
        }
        let Some([a, b, c, ..]) = change.state.lines.get(start..) else {
            continue;
        };
        let key = (
            change.mode.normalize(&a.text),
            change.mode.normalize(&b.text),
            change.mode.normalize(&c.text),
        );
        let Some(candidates) = starts.get(&key) else {
            continue;
        };
        // Candidate lists retain old positions in order, preserving tie-breaking.
        for &old_start in candidates {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AuthorId, FileState, NormalizationMode, Policy, SCALE, adopt_parent, apply_change,
    };

    /// Build scored states and changes under the ordinary line-level policy.
    fn change(previous: &FileState, text: &str, author: u32) -> Change {
        apply_change(
            previous,
            text,
            &[(AuthorId(author), SCALE)],
            NormalizationMode::IgnoreIndentation,
            Policy::default(),
        )
        .unwrap()
    }

    #[test]
    fn adopted_removals_and_parent_union_refund_only_one_destination() {
        let original = change(&FileState::default(), "one\ntwo\nthree\nkeep\n", 0);
        let parent = change(&original.state, "keep\n", 1);
        let mut adopted = change(&original.state, "keep\n", 2);
        adopt_parent(&mut adopted, &parent.state);
        assert_eq!(adopted.labor, std::collections::BTreeMap::new());
        let mut staged = vec![
            ("a.rs".into(), adopted),
            (
                "b.rs".into(),
                change(&FileState::default(), "one\ntwo\nthree\n", 2),
            ),
            (
                "c.rs".into(),
                change(&FileState::default(), "one\ntwo\nthree\n", 2),
            ),
        ];
        assert_eq!(
            reconcile_merge_moves(&mut staged, &[vec![("a.rs".into(), parent)]]),
            3
        );
        assert_eq!(
            staged[1].1.state.lines[0].owner,
            original.state.lines[0].owner
        );
        assert_eq!(staged[1].1.labor[&AuthorId(2)], 0);
        assert_eq!(staged[2].1.labor[&AuthorId(2)], 3 * u64::from(SCALE));
    }

    #[test]
    fn other_parent_supplies_sources_without_changing_its_ledger() {
        let original = change(&FileState::default(), "one\ntwo\nthree\n", 0);
        let removed = change(&original.state, "", 1);
        let mut staged = vec![(
            "b.rs".into(),
            change(&FileState::default(), "one\ntwo\nthree\n", 2),
        )];
        assert_eq!(
            reconcile_merge_moves(&mut staged, &[vec![("a.rs".into(), removed)]]),
            3
        );
        assert_eq!(
            staged[0].1.state.lines[0].owner,
            original.state.lines[0].owner
        );
        assert_eq!(staged[0].1.labor[&AuthorId(2)], 0);
    }
}
