//! Token provenance and exact line-equivalent accounting, independent of replay.

use std::collections::{BTreeMap, BTreeSet};

use similar::{Algorithm, DiffTag, capture_diff_slices};

use crate::model::add_totals;
use crate::tokenize;
use crate::{AuthorId, ChangeKind, Error, LaborDelta, LineOwner, NormalizationMode, SCALE};

/// Owned canonical token identity. Positions and weights never affect alignment.
type Key = (Option<usize>, String);
/// An old-to-new canonical spelling replacement, independent of file position.
type Mapping = (Key, Key);

/// Provenance of one meaningful lexical unit. Zero-weight commas are omitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedToken {
    key: Key,
    line: usize,
    weight: u16,
    owner: LineOwner,
}

impl OwnedToken {
    /// Canonical identity: `(None, spelling)` for text, `(Some(width), "")` for
    /// indentation. Lexical category does not affect unchanged-text provenance.
    pub fn key(&self) -> (Option<usize>, &str) {
        (self.key.0, &self.key.1)
    }

    /// Physical zero-based line position in the current file.
    pub fn line(&self) -> usize {
        self.line
    }

    /// Lexical weight in per-mille units, before line-equivalent normalization.
    pub fn weight(&self) -> u16 {
        self.weight
    }

    /// Conserved token provenance, in millionths shared among its authors.
    pub fn owner(&self) -> &LineOwner {
        &self.owner
    }
}

/// Live token provenance. Construction is restricted to validated attribution so
/// callers cannot inject inconsistent weights, positions or author shares.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenState {
    tokens: Vec<OwnedToken>,
}

impl TokenState {
    /// Meaningful tokens in physical file order; zero-weight commas are omitted.
    pub fn tokens(&self) -> &[OwnedToken] {
        &self.tokens
    }

    /// Count physical lines containing at least one positive-weight token.
    pub fn meaningful_lines(&self) -> usize {
        line_ranges(&self.tokens).count()
    }

    /// Apportion exactly one ownership unit per meaningful physical line.
    /// Weighted author shares are aggregated before rounding, in author-ID order.
    /// Cumulative integer division conserves the total exactly and gives each
    /// author less than one millionth of a line of rounding error per line.
    ///
    /// # Errors
    /// Returns `Overflow` when an accumulated ownership total cannot fit in `u64`.
    pub fn ownership(&self) -> Result<LaborDelta, Error> {
        let mut ownership = LaborDelta::new();
        for line in line_ranges(&self.tokens) {
            let mut weighted: BTreeMap<AuthorId, u128> = BTreeMap::new();
            let mut total_weight = 0_u128;
            for token in line {
                total_weight += u128::from(token.weight);
                for &(author, share) in token.owner.authors() {
                    *weighted.entry(author).or_default() +=
                        u128::from(token.weight) * u128::from(share);
                }
            }
            let mut cumulative = 0;
            let mut previous = 0;
            let mut delta = LaborDelta::new();
            for (author, weight) in weighted {
                cumulative += weight;
                let next = cumulative / total_weight;
                let share = u64::try_from(next - previous).map_err(|_| Error::Overflow)?;
                if share > 0 {
                    delta.insert(author, share);
                }
                previous = next;
            }
            add_totals(&mut ownership, &delta)?;
        }
        Ok(ownership)
    }
}

/// A paired replacement eligible for commit-wide bulk-rename analysis.
#[derive(Debug, Clone)]
struct Replacement {
    old: OwnedToken,
    new: usize,
}

/// A staged token change. Reconcile the complete commit before accumulating its
/// labor. This foundation does not yet implement merge or move reconciliation.
#[derive(Debug, Clone)]
pub struct TokenChange {
    state: TokenState,
    editor: LineOwner,
    inserted: BTreeSet<usize>,
    replacements: Vec<Replacement>,
    replaced_weight: u128,
    bulk_mappings: BTreeSet<Mapping>,
    kind: ChangeKind,
}

impl TokenChange {
    /// New live provenance, after any bulk-rename reconciliation.
    pub fn state(&self) -> &TokenState {
        &self.state
    }

    /// Whether the positive-weight canonical token sequence changed.
    pub fn kind(&self) -> ChangeKind {
        self.kind
    }

    /// Move the validated live provenance into the caller's next file version.
    pub fn into_state(self) -> TokenState {
        self.state
    }

    /// Compute inserted-weight fractions of each output line, rounded downward
    /// to millionths, then split among editors with conserved cumulative rounding.
    /// Bulk mappings add exactly one line each in this file, independent of their
    /// occurrence count. Deleted-only edits earn zero labor.
    ///
    /// # Errors
    /// Returns `Overflow` if the historical delta cannot be represented in `u64`.
    pub fn labor(&self) -> Result<LaborDelta, Error> {
        let mut labor = LaborDelta::new();
        let mut start = 0;
        for line in line_ranges(&self.state.tokens) {
            let mut inserted = 0_u128;
            let mut total = 0_u128;
            for (offset, token) in line.iter().enumerate() {
                total += u128::from(token.weight);
                if self.inserted.contains(&(start + offset)) {
                    inserted += u128::from(token.weight);
                }
            }
            let amount =
                u32::try_from(inserted * u128::from(SCALE) / total).map_err(|_| Error::Overflow)?;
            if amount > 0 {
                add_totals(&mut labor, &self.editor.labor(amount))?;
            }
            start += line.len();
        }
        for _ in &self.bulk_mappings {
            add_totals(&mut labor, &self.editor.labor(SCALE))?;
        }
        Ok(labor)
    }
}

/// Diff canonical token IDs with deterministic Myers, retaining equal-token
/// provenance and assigning inserted/replaced tokens to the validated editors.
/// Whitespace, line wrapping and zero-weight trailing commas do not participate
/// in alignment. The existing line engine and its keep threshold are independent.
///
/// This API is a pure Phase 2b foundation, not yet used by repository replay.
/// It does not reconcile moves or merges; callers must not substitute it for the
/// line engine in merge-aware replay without implementing those steps.
///
/// # Errors
/// Returns `InvalidShares` for empty, duplicate, zero or nonconserving authors.
pub fn apply_token_change(
    previous: &TokenState,
    text: &str,
    authors: &[(AuthorId, u32)],
    mode: NormalizationMode,
) -> Result<TokenChange, Error> {
    let editor = LineOwner::new(authors)?;
    let tokens = tokenize(text, mode)
        .into_iter()
        .filter(|token| token.weight > 0)
        .map(|token| OwnedToken {
            key: (token.key().0, token.key().1.to_owned()),
            line: token.line,
            weight: token.weight,
            owner: editor.clone(),
        })
        .collect::<Vec<_>>();
    // Intern both streams together. IDs are assigned by first encounter, not by
    // hash order, and Myers has no time-based deadline or approximate fallback.
    let mut interned = BTreeMap::new();
    let mut intern = |token: &OwnedToken| {
        let next = interned.len();
        *interned.entry(token.key.clone()).or_insert(next)
    };
    let old: Vec<_> = previous.tokens.iter().map(&mut intern).collect();
    let new: Vec<_> = tokens.iter().map(&mut intern).collect();
    let mut result = TokenChange {
        state: TokenState { tokens },
        editor,
        inserted: BTreeSet::new(),
        replacements: Vec::new(),
        replaced_weight: 0,
        bulk_mappings: BTreeSet::new(),
        kind: if old == new {
            ChangeKind::Formatting
        } else {
            ChangeKind::Meaningful
        },
    };
    for op in capture_diff_slices(Algorithm::Myers, &old, &new) {
        let before = previous.tokens.get(op.old_range()).unwrap_or_default();
        if op.tag() == DiffTag::Equal {
            for (source, index) in before.iter().zip(op.new_range()) {
                if let Some(token) = result.state.tokens.get_mut(index) {
                    token.owner = source.owner.clone();
                }
            }
        } else {
            result.inserted.extend(op.new_range());
            if op.tag() == DiffTag::Replace {
                result.replaced_weight += result
                    .state
                    .tokens
                    .get(op.new_range())
                    .unwrap_or_default()
                    .iter()
                    .map(|token| u128::from(token.weight))
                    .sum::<u128>();
                // Unequal hunks are not guessed into mappings. Myers' equal
                // anchors already isolate aligned single-token replacements.
                if op.old_range().len() == op.new_range().len() {
                    result
                        .replacements
                        .extend(
                            before
                                .iter()
                                .zip(op.new_range())
                                .map(|(old, new)| Replacement {
                                    old: old.clone(),
                                    new,
                                }),
                        );
                }
            }
        }
    }
    Ok(result)
}

/// Detect dominant repeated mappings over all files of one commit, restore the
/// renamed tokens' owners and replace their insertion labor with one line per
/// mapping per file. A mapping needs at least five occurrences and at least 60%
/// of all replaced new-token weight, including unpaired replacement hunks.
///
/// Pass each file exactly once. Disabling the discount leaves all state untouched.
/// Repeated calls with the discount enabled are idempotent. Returns whether this
/// call found any qualifying mapping. This does not produce commit IDs or warnings;
/// those belong to repository replay and reporting.
pub fn discount_bulk_edits(changes: &mut [TokenChange], enabled: bool) -> bool {
    if !enabled {
        return false;
    }
    let mut counts: BTreeMap<Mapping, (usize, u128)> = BTreeMap::new();
    let total: u128 = changes.iter().map(|change| change.replaced_weight).sum();
    for change in changes.iter() {
        for replacement in &change.replacements {
            if let Some(new) = change.state.tokens.get(replacement.new) {
                let entry = counts
                    .entry((replacement.old.key.clone(), new.key.clone()))
                    .or_default();
                entry.0 += 1;
                entry.1 += u128::from(new.weight);
            }
        }
    }
    let mappings: BTreeSet<_> = counts
        .into_iter()
        .filter_map(|(mapping, (count, weight))| {
            (count >= 5 && weight * 100 >= total * 60).then_some(mapping)
        })
        .collect();
    for change in changes {
        for replacement in &change.replacements {
            if let Some(new) = change.state.tokens.get_mut(replacement.new) {
                let mapping = (replacement.old.key.clone(), new.key.clone());
                if mappings.contains(&mapping) {
                    new.owner = replacement.old.owner.clone();
                    change.inserted.remove(&replacement.new);
                    change.bulk_mappings.insert(mapping);
                }
            }
        }
    }
    !mappings.is_empty()
}

/// Group a validated, ordered stream into nonempty physical line slices.
fn line_ranges(tokens: &[OwnedToken]) -> impl Iterator<Item = &[OwnedToken]> {
    tokens.chunk_by(|left, right| left.line == right.line)
}

#[cfg(test)]
#[path = "token_tests.rs"]
mod tests;
