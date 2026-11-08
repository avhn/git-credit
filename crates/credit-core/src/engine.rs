//! Deterministic normalized line diffs and bounded replacement pairing.

use std::collections::BTreeMap;

use similar::{Algorithm, DiffTag, capture_diff_slices};

use crate::model::add_totals;
use crate::{Error, FileState, LaborDelta, Line, LineOwner, NormalizationMode, Policy, SCALE};

/// Maximum candidate pairs per replacement hunk before treating it as a rewrite.
const MAX_PAIRS: usize = 250_000;

/// Classification of a file change, independent of subsequent move reconciliation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// Normalized line sequences are equal, including unchanged and empty files.
    Formatting,
    /// At least one normalized line was inserted, deleted or edited.
    Meaningful,
}

/// Result of one file diff, with transient metadata for same-commit moves.
#[derive(Debug, Clone)]
pub struct Change {
    /// New live attribution, including original (unnormalized) line text.
    pub state: FileState,
    /// Editor labor in millionths of a line. Reconcile moves before accumulating.
    pub labor: LaborDelta,
    /// Whether normalized sequences differ.
    pub kind: ChangeKind,
    /// True if any replacement hunk exceeded the pairing cap.
    pub pairing_capped: bool,
    pub(crate) removed: Vec<Vec<Line>>,
    pub(crate) added: BTreeMap<usize, LaborDelta>,
    pub(crate) mode: NormalizationMode,
}

/// Compare a previous file (empty for a new file) with new text using Myers.
/// New lines greedily select the most similar unused old line in their replace
/// hunk; equal scores choose the earliest old position. Similarity uses Unicode
/// scalar values: twice the number of equal characters divided by combined length.
/// Threshold comparisons are exact; fractional labor rounds upward to millionths.
/// Hunk products above 250,000 skip pairing and set `pairing_capped`.
///
/// # Errors
/// Returns `InvalidThreshold`, `InvalidShares`, or `Overflow` for invalid policy,
/// invalid commit authors, or unrepresentable labor respectively. Inputs are not
/// mutated. Deletions generate no labor; their provenance remains available for moves.
///
/// # Examples
/// ```
/// use credit_core::{AuthorId, FileState, NormalizationMode, Policy, SCALE, apply_change};
/// let result = apply_change(&FileState::default(), "hello\n", &[(AuthorId(0), SCALE)],
///     NormalizationMode::IgnoreIndentation, Policy::default())?;
/// assert_eq!(result.labor[&AuthorId(0)], u64::from(SCALE));
/// # Ok::<(), credit_core::Error>(())
/// ```
pub fn apply_change(
    prev: &FileState,
    new_text: &str,
    commit_authors: &[(crate::AuthorId, u32)],
    mode: NormalizationMode,
    policy: Policy,
) -> Result<Change, Error> {
    policy.validate()?;
    let editor = LineOwner::new(commit_authors)?;
    let raw: Vec<_> = new_text.lines().collect();
    let old: Vec<_> = prev
        .lines
        .iter()
        .map(|line| mode.normalize(&line.text))
        .collect();
    let new: Vec<_> = raw.iter().map(|line| mode.normalize(line)).collect();
    let mut result = Change {
        state: FileState::default(),
        labor: BTreeMap::new(),
        kind: if old == new {
            ChangeKind::Formatting
        } else {
            ChangeKind::Meaningful
        },
        pairing_capped: false,
        removed: Vec::new(),
        added: BTreeMap::new(),
        mode,
    };
    for op in capture_diff_slices(Algorithm::Myers, &old, &new) {
        let old_lines = prev.lines.get(op.old_range()).unwrap_or_default();
        let new_lines = raw.get(op.new_range()).unwrap_or_default();
        if op.tag() == DiffTag::Equal {
            for (line, text) in old_lines.iter().zip(new_lines) {
                result.state.lines.push(Line {
                    text: (*text).to_owned(),
                    owner: line.owner.clone(),
                });
            }
        } else {
            apply_hunk(&mut result, old_lines, new_lines, &editor, policy)?;
        }
    }
    Ok(result)
}

/// Attribute a replacement, insertion or deletion while preserving unmatched
/// removed blocks. Pairing is skipped as a whole when the hunk exceeds its cap.
fn apply_hunk(
    result: &mut Change,
    old: &[Line],
    new: &[&str],
    editor: &LineOwner,
    policy: Policy,
) -> Result<(), Error> {
    let capped = old
        .len()
        .checked_mul(new.len())
        .is_none_or(|pairs| pairs > MAX_PAIRS);
    result.pairing_capped |= capped;
    let mut used = vec![false; old.len()];
    let mut retained = vec![false; old.len()];
    for text in new {
        let candidate = if capped {
            None
        } else {
            best_match(old, &used, result.mode.normalize(text), result.mode)
        };
        let mut owner = editor.clone();
        let mut labor = SCALE;
        let mut inserted = true;
        if let Some((index, score)) = candidate {
            if let Some(slot) = used.get_mut(index) {
                *slot = true;
            }
            if score.numerator * 1000 >= score.denominator * u128::from(policy.keep_per_mille)
                && let Some(previous) = old.get(index)
            {
                owner = previous.owner.clone();
                labor = SCALE - score.parts();
                inserted = false;
                if let Some(slot) = retained.get_mut(index) {
                    *slot = true;
                }
            }
        }
        let delta = editor.labor(labor);
        add_totals(&mut result.labor, &delta)?;
        if inserted {
            result.added.insert(result.state.lines.len(), delta);
        }
        result.state.lines.push(Line {
            text: (*text).to_owned(),
            owner,
        });
    }
    let mut block = Vec::new();
    for (line, kept) in old.iter().zip(retained) {
        if kept {
            if !block.is_empty() {
                result.removed.push(std::mem::take(&mut block));
            }
        } else {
            block.push(line.clone());
        }
    }
    if !block.is_empty() {
        result.removed.push(block);
    }
    Ok(())
}

/// Rational character similarity, avoiding floating-point threshold drift.
#[derive(Clone, Copy)]
struct Similarity {
    numerator: u128,
    denominator: u128,
}

impl Similarity {
    /// Quantize similarity downward; the complementary labor rounds upward.
    fn parts(self) -> u32 {
        u32::try_from(self.numerator * u128::from(SCALE) / self.denominator).unwrap_or(SCALE)
    }
}

/// Select the earliest maximum without sorting or allocating a quadratic matrix.
fn best_match(
    old: &[Line],
    used: &[bool],
    text: &str,
    mode: NormalizationMode,
) -> Option<(usize, Similarity)> {
    let new_chars: Vec<_> = text.chars().collect();
    let mut best: Option<(usize, Similarity)> = None;
    for (index, (line, used)) in old.iter().zip(used).enumerate() {
        if *used {
            continue;
        }
        let old_chars: Vec<_> = mode.normalize(&line.text).chars().collect();
        let denominator = old_chars.len() as u128 + new_chars.len() as u128;
        let equal: usize = capture_diff_slices(Algorithm::Myers, &old_chars, &new_chars)
            .iter()
            .filter(|op| op.tag() == DiffTag::Equal)
            .map(|op| op.old_range().len())
            .sum();
        let score = if denominator == 0 {
            Similarity {
                numerator: 1,
                denominator: 1,
            }
        } else {
            Similarity {
                numerator: 2 * equal as u128,
                denominator,
            }
        };
        if best.is_none_or(|(_, current)| {
            score.numerator * current.denominator > current.numerator * score.denominator
        }) {
            best = Some((index, score));
        }
    }
    best
}
