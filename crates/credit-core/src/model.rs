//! Validated attribution values and fixed-point accounting.

use std::collections::{BTreeMap, BTreeSet};

use crate::Error;

/// Fixed-point units per line, for both ownership and labor.
pub const SCALE: u32 = 1_000_000;

/// Index into the caller's identity table; the engine never interprets identities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AuthorId(pub u32);

/// Validated ownership shares, in millionths of a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineOwner {
    authors: Vec<(AuthorId, u32)>,
}

impl LineOwner {
    /// Validate explicit shares. Order is preserved for deterministic rounding.
    ///
    /// # Errors
    /// Returns `InvalidShares` for empty, duplicate, zero or nonconserving shares.
    pub fn new(authors: &[(AuthorId, u32)]) -> Result<Self, Error> {
        let mut seen = BTreeSet::new();
        let mut sum = 0_u64;
        for &(id, share) in authors {
            if share == 0 || !seen.insert(id) {
                return Err(Error::InvalidShares);
            }
            sum += u64::from(share);
        }
        if sum != u64::from(SCALE) {
            return Err(Error::InvalidShares);
        }
        Ok(Self {
            authors: authors.to_vec(),
        })
    }

    /// Split a line equally among distinct authors in input order.
    /// Remainder units go to the earliest authors so shares sum exactly to `SCALE`.
    ///
    /// # Errors
    /// Rejects no authors, duplicate authors, or more than `SCALE` authors.
    pub fn equal(authors: &[AuthorId]) -> Result<Self, Error> {
        let count = u32::try_from(authors.len()).map_err(|_| Error::InvalidShares)?;
        if count == 0 || count > SCALE {
            return Err(Error::InvalidShares);
        }
        let shares: Vec<_> = authors
            .iter()
            .zip(0..count)
            .map(|(&id, i)| (id, SCALE / count + u32::from(i < SCALE % count)))
            .collect();
        Self::new(&shares)
    }

    /// Return shares in their stable rounding order.
    pub fn authors(&self) -> &[(AuthorId, u32)] {
        &self.authors
    }

    /// Apportion fractional labor by cumulative integer division, preserving its
    /// exact total. Rounding differences are at most one millionth per author.
    pub(crate) fn labor(&self, amount: u32) -> LaborDelta {
        let mut cumulative = 0_u64;
        let mut previous = 0_u64;
        let mut result = BTreeMap::new();
        for &(id, share) in &self.authors {
            cumulative += u64::from(share) * u64::from(amount);
            let next = cumulative / u64::from(SCALE);
            result.insert(id, next - previous);
            previous = next;
        }
        result
    }
}

/// Caller-selected whitespace semantics. Unknown languages should be significant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalizationMode {
    /// Preserve leading whitespace, for Python, YAML, Makefiles and unknown files.
    IndentationSignificant,
    /// Ignore leading whitespace where indentation has no language meaning.
    IgnoreIndentation,
}

impl NormalizationMode {
    /// Remove trailing Unicode whitespace (including CR), optionally leading
    /// whitespace. Interior whitespace and all non-whitespace characters survive.
    pub fn normalize(self, text: &str) -> &str {
        match self {
            Self::IndentationSignificant => text.trim_end(),
            Self::IgnoreIndentation => text.trim(),
        }
    }
}

/// Similarity threshold. Validation happens at the engine's public boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// Minimum character similarity, in 0..=1000, for retaining old ownership.
    pub keep_per_mille: u32,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            keep_per_mille: 700,
        }
    }
}

impl Policy {
    /// Validate the configured keep threshold.
    ///
    /// # Errors
    /// Returns `InvalidThreshold` when the threshold exceeds 1000.
    pub fn validate(self) -> Result<(), Error> {
        if self.keep_per_mille > 1000 {
            return Err(Error::InvalidThreshold {
                name: "keep",
                per_mille: self.keep_per_mille,
            });
        }
        Ok(())
    }
}

/// A physical line, without its LF terminator, with conserved ownership.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// Original text, retained so future normalization modes can be applied safely.
    pub text: String,
    /// Attribution of this line.
    pub owner: LineOwner,
}

/// Live attribution for one file. Empty files contain no lines.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileState {
    /// Physical lines in file order, including blank lines.
    pub lines: Vec<Line>,
}

/// Per-author labor in millionths of a line, ordered by caller-assigned ID.
pub type LaborDelta = BTreeMap<AuthorId, u64>;

/// Ownership of live content and accumulated historical labor, in millionths.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ledger {
    /// Current ownership; recompute after deletions, renames and moves.
    pub ownership: BTreeMap<AuthorId, u64>,
    /// Historical labor, never reduced by subsequent commits.
    pub labor: LaborDelta,
}

impl Ledger {
    /// Add a completed commit's labor atomically.
    ///
    /// # Errors
    /// Returns `Overflow` without changing the ledger when a total exceeds `u64`.
    pub fn add_labor(&mut self, delta: &LaborDelta) -> Result<(), Error> {
        let mut updated = self.labor.clone();
        add_totals(&mut updated, delta)?;
        self.labor = updated;
        Ok(())
    }

    /// Replace ownership with the sum over all live files, without changing labor.
    ///
    /// # Errors
    /// Returns `Overflow` without changing ownership if a total exceeds `u64`.
    pub fn recount<'a>(
        &mut self,
        files: impl IntoIterator<Item = &'a FileState>,
    ) -> Result<(), Error> {
        let mut ownership = BTreeMap::new();
        for file in files {
            for line in &file.lines {
                if line.text.trim().is_empty() {
                    continue;
                }
                for &(id, share) in line.owner.authors() {
                    add_total(&mut ownership, id, u64::from(share))?;
                }
            }
        }
        self.ownership = ownership;
        Ok(())
    }
}

/// Add one contribution without wrapping fixed-point arithmetic.
fn add_total(totals: &mut LaborDelta, id: AuthorId, amount: u64) -> Result<(), Error> {
    let entry = totals.entry(id).or_default();
    *entry = entry.checked_add(amount).ok_or(Error::Overflow)?;
    Ok(())
}

/// Accumulate one map into another, checking each addition.
pub(crate) fn add_totals(totals: &mut LaborDelta, delta: &LaborDelta) -> Result<(), Error> {
    for (&id, &amount) in delta {
        add_total(totals, id, amount)?;
    }
    Ok(())
}
