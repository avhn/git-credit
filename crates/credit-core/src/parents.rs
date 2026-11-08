//! Exact normalized parent provenance for merge insertions and rewrites.

use std::collections::BTreeMap;

use crate::{Change, FileState};

/// Adopt an exact parent blob without discarding first-parent removal provenance.
/// The caller must verify blob equality. Destination ownership is replaced and
/// all editor labor and insertion candidates are cleared; removed blocks remain
/// available to cross-file move reconciliation.
pub fn adopt_parent(change: &mut Change, parent: &FileState) {
    change.state = parent.clone();
    change.labor.clear();
    change.added.clear();
}

/// Restore parent ownership for editor-inserted or rewritten lines and refund
/// their exact recorded labor shares. Retained and small-edit lines are untouched.
/// Parents are searched in supplied order, then physical line order; repeated
/// text uses the first matching owner. Blank lines are not matched. Call before
/// move reconciliation and before accumulating labor. Repeated calls are safe.
/// Returns the number of meaningful lines reassigned.
pub fn reconcile_parents<'a>(
    change: &mut Change,
    parents: impl IntoIterator<Item = &'a FileState>,
) -> usize {
    let mut owners = BTreeMap::new();
    for parent in parents {
        for line in &parent.lines {
            let text = change.mode.normalize(&line.text);
            if !text.is_empty() {
                owners.entry(text).or_insert(&line.owner);
            }
        }
    }
    let mut matched = Vec::new();
    for &position in change.added.keys() {
        if let Some(line) = change.state.lines.get_mut(position)
            && let Some(owner) = owners.get(change.mode.normalize(&line.text))
        {
            line.owner = (*owner).clone();
            matched.push(position);
        }
    }
    for position in &matched {
        if let Some(delta) = change.added.remove(position) {
            for (id, amount) in delta {
                if let Some(total) = change.labor.get_mut(&id) {
                    // Private candidate metadata records a subset of this total.
                    *total -= amount;
                }
            }
        }
    }
    matched.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AuthorId, NormalizationMode, Policy, SCALE, apply_change};

    #[test]
    fn refunds_exact_coauthor_shares_and_is_idempotent() {
        let mode = NormalizationMode::IgnoreIndentation;
        let parent = apply_change(
            &FileState::default(),
            "inherited\n",
            &[(AuthorId(0), SCALE)],
            mode,
            Policy::default(),
        )
        .unwrap();
        let mut merge = apply_change(
            &FileState::default(),
            "  inherited\nresolution\n\n",
            &[(AuthorId(1), 333_333), (AuthorId(2), 666_667)],
            mode,
            Policy::default(),
        )
        .unwrap();
        assert_eq!(reconcile_parents(&mut merge, [&parent.state]), 1);
        assert_eq!(merge.state.lines[0].owner, parent.state.lines[0].owner);
        assert_eq!(merge.labor[&AuthorId(1)], 333_333);
        assert_eq!(merge.labor[&AuthorId(2)], 666_667);
        assert_eq!(reconcile_parents(&mut merge, [&parent.state]), 0);
    }

    #[test]
    fn retains_small_edits_and_uses_first_parent_match() {
        let mode = NormalizationMode::IgnoreIndentation;
        let old = apply_change(
            &FileState::default(),
            "a long existing line\n",
            &[(AuthorId(0), SCALE)],
            mode,
            Policy::default(),
        )
        .unwrap();
        let parent = apply_change(
            &FileState::default(),
            "a long existing line!\nnew\n",
            &[(AuthorId(1), SCALE)],
            mode,
            Policy::default(),
        )
        .unwrap();
        let other = apply_change(
            &FileState::default(),
            "new\n",
            &[(AuthorId(2), SCALE)],
            mode,
            Policy::default(),
        )
        .unwrap();
        let mut merge = apply_change(
            &old.state,
            "a long existing line!\nnew\n",
            &[(AuthorId(3), SCALE)],
            mode,
            Policy::default(),
        )
        .unwrap();
        assert_eq!(
            reconcile_parents(&mut merge, [&parent.state, &other.state]),
            1
        );
        assert_eq!(merge.state.lines[0].owner, old.state.lines[0].owner);
        assert_eq!(merge.state.lines[1].owner, parent.state.lines[1].owner);
        assert!(merge.labor[&AuthorId(3)] > 0);
        assert!(merge.labor[&AuthorId(3)] < u64::from(SCALE));
    }
}
