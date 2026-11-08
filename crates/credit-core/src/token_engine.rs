//! Token provenance and exact line-equivalent accounting, independent of replay.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    hash::{BuildHasherDefault, DefaultHasher},
    sync::Arc,
};

use similar::{Algorithm, DiffTag, capture_diff_slices};

use crate::model::add_totals;
use crate::tokenize;
use crate::{AuthorId, ChangeKind, Error, LaborDelta, LineOwner, NormalizationMode, SCALE};

/// Owned canonical token identity. Positions and weights never affect alignment.
type Key = (Option<usize>, Arc<str>);
/// An old-to-new canonical spelling replacement, independent of file position.
type Mapping = (Key, Key);

/// Provenance of one meaningful lexical unit. Zero-weight commas are omitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedToken {
    key: Key,
    line: usize,
    first_in_line: bool,
    last_in_line: bool,
    identifier_word: bool,
    weight: u16,
    owner: Arc<LineOwner>,
}

impl OwnedToken {
    /// Restrict bulk renames to code identifiers. Defer the keyword lookup until
    /// a replacement is examined; unchanged tokens only store lexical context.
    fn rename_candidate(&self) -> bool {
        self.identifier_word && crate::lexer::identifier_name(&self.key.1)
    }

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

    /// Restore validated provenance onto an already lexed file state.
    /// Token identities, boundaries and weights remain unchanged.
    ///
    /// # Errors
    /// Returns `InvalidShares` if the owner count differs from the token count.
    pub fn restore_owners(&mut self, owners: Vec<LineOwner>) -> Result<(), Error> {
        if owners.len() != self.tokens.len() {
            return Err(Error::InvalidShares);
        }
        for (token, owner) in self.tokens.iter_mut().zip(owners) {
            token.owner = Arc::new(owner);
        }
        Ok(())
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
/// labor. Parent and move reconciliation precede bulk detection and accounting.
#[derive(Debug, Clone)]
pub struct TokenChange {
    state: TokenState,
    editor: LineOwner,
    inserted: BTreeSet<usize>,
    replacements: Vec<Replacement>,
    replaced: BTreeSet<usize>,
    moves_reconciled: bool,
    bulk_mappings: BTreeSet<Mapping>,
    kind: ChangeKind,
    removed: Vec<Vec<OwnedToken>>,
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
/// Reconcile parent provenance, cross-file moves and bulk mappings before
/// accumulating labor. Removed runs remain available after exact parent adoption.
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
    // Intern once across both versions, borrowing lookup keys and sharing owned
    // spellings/provenance. This avoids allocating an owner vector and duplicate
    // canonical strings for every token in every replayed file version.
    // Never iterate this table: IDs follow encounter order. Explicit hashing
    // avoids random seeding (and hidden entropy I/O) in the pure core.
    let mut symbols = HashMap::with_hasher(BuildHasherDefault::<DefaultHasher>::default());
    let old: Vec<_> = previous
        .tokens
        .iter()
        .map(|token| {
            let next = symbols.len();
            symbols
                .entry(token.key())
                .or_insert_with(|| (next, Arc::clone(&token.key.1)))
                .0
        })
        .collect();
    let old_symbols = symbols.len();
    let mut new = Vec::new();
    let mut shared = false;
    let owner = Arc::new(editor.clone());
    let mut tokens = tokenize(text, mode)
        .into_iter()
        .filter(|token| token.weight > 0)
        .map(|token| {
            let next = symbols.len();
            let (id, spelling) = symbols
                .entry(token.key())
                .or_insert_with(|| (next, Arc::<str>::from(token.key().1)));
            shared |= *id < old_symbols;
            new.push(*id);
            OwnedToken {
                key: (token.key().0, Arc::clone(spelling)),
                line: token.line,
                first_in_line: false,
                last_in_line: false,
                identifier_word: token.kind == crate::TokenKind::Word,
                weight: token.weight,
                owner: Arc::clone(&owner),
            }
        })
        .collect::<Vec<_>>();
    for line in tokens.chunk_by_mut(|a, b| a.line == b.line) {
        if let Some(first) = line.first_mut() {
            first.first_in_line = true;
        }
        if let Some(last) = line.last_mut() {
            last.last_in_line = true;
        }
    }
    let mut result = TokenChange {
        state: TokenState { tokens },
        editor,
        inserted: BTreeSet::new(),
        replacements: Vec::new(),
        replaced: BTreeSet::new(),
        moves_reconciled: false,
        bulk_mappings: BTreeSet::new(),
        removed: Vec::new(),
        kind: if old == new {
            ChangeKind::Formatting
        } else {
            ChangeKind::Meaningful
        },
    };
    // Disjoint vocabularies have exactly one replacement and no provenance to
    // align. Avoid Myers' quadratic search without changing its edit script.
    let operations = if !shared && !old.is_empty() && !new.is_empty() {
        vec![similar::DiffOp::Replace {
            old_index: 0,
            old_len: old.len(),
            new_index: 0,
            new_len: new.len(),
        }]
    } else {
        capture_diff_slices(Algorithm::Myers, &old, &new)
    };
    apply_operations(previous, &mut result, operations);
    Ok(result)
}

/// Preserve equal provenance and stage removal, insertion and replacement metadata.
fn apply_operations(
    previous: &TokenState,
    result: &mut TokenChange,
    operations: Vec<similar::DiffOp>,
) {
    for op in operations {
        let before = previous.tokens.get(op.old_range()).unwrap_or_default();
        if op.tag() == DiffTag::Equal {
            for (source, index) in before.iter().zip(op.new_range()) {
                if let Some(token) = result.state.tokens.get_mut(index) {
                    token.owner = source.owner.clone();
                }
            }
        } else {
            if !before.is_empty() {
                result.removed.push(before.to_vec());
            }
            result.inserted.extend(op.new_range());
            if op.tag() == DiffTag::Replace {
                result.replaced.extend(op.new_range());
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
}

/// Detect dominant repeated mappings over all files of one commit, restore the
/// renamed tokens' owners and replace their insertion labor with one line per
/// mapping per file. A mapping needs at least five occurrences and at least 60%
/// of all replaced new-token weight, including unpaired replacement hunks. Only
/// identifier-to-identifier pairs qualify; comments, literals, keywords, operators
/// and indentation remain ordinary edits and still contribute to the denominator.
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
    let total: u128 = changes
        .iter()
        .flat_map(|change| {
            change
                .replaced
                .iter()
                .filter_map(|index| change.state.tokens.get(*index))
        })
        .map(|token| u128::from(token.weight))
        .sum();
    for change in changes.iter() {
        for replacement in &change.replacements {
            if let Some(new) = change.state.tokens.get(replacement.new)
                && replacement.old.rename_candidate()
                && new.rename_candidate()
                && (change.inserted.contains(&replacement.new)
                    || change
                        .bulk_mappings
                        .contains(&(replacement.old.key.clone(), new.key.clone())))
            {
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
                if replacement.old.rename_candidate()
                    && new.rename_candidate()
                    && mappings.contains(&mapping)
                    && (change.inserted.contains(&replacement.new)
                        || change.bulk_mappings.contains(&mapping))
                {
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

#[path = "token_reconcile.rs"]
mod reconciliation;
pub use reconciliation::{adopt_token_parent, reconcile_token_moves, reconcile_token_parents};
