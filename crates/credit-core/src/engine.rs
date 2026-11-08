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
    /// Normalized nonblank sequences are equal, including unchanged and empty files.
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
    let old_positions: Vec<_> = prev
        .lines
        .iter()
        .enumerate()
        .filter(|(_, line)| !mode.normalize(&line.text).is_empty())
        .collect();
    let new_positions: Vec<_> = raw
        .iter()
        .enumerate()
        .filter(|(_, line)| !mode.normalize(line).is_empty())
        .collect();
    let old: Vec<_> = old_positions
        .iter()
        .map(|(_, line)| mode.normalize(&line.text))
        .collect();
    let new: Vec<_> = new_positions
        .iter()
        .map(|(_, line)| mode.normalize(line))
        .collect();
    let mut retained = vec![false; prev.lines.len()];
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
    // Blank runs never participate in alignment, including mixed edits. Keep
    // physical positions separately so moves still require consecutive lines.
    for op in capture_diff_slices(Algorithm::Myers, &old, &new) {
        let old_entries = old_positions.get(op.old_range()).unwrap_or_default();
        let new_entries = new_positions.get(op.new_range()).unwrap_or_default();
        let kept = if op.tag() == DiffTag::Equal {
            for ((_, line), (_, text)) in old_entries.iter().zip(new_entries) {
                result.state.lines.push(Line {
                    text: (**text).to_owned(),
                    owner: line.owner.clone(),
                });
            }
            vec![true; old_entries.len()]
        } else {
            let old_lines: Vec<_> = old_entries.iter().map(|(_, line)| *line).collect();
            let new_lines: Vec<_> = new_entries.iter().map(|(_, text)| **text).collect();
            apply_hunk(&mut result, &old_lines, &new_lines, &editor, policy)?
        };
        for ((physical, _), kept) in old_entries.iter().zip(kept) {
            if let Some(slot) = retained.get_mut(*physical) {
                *slot = kept;
            }
        }
    }
    restore_physical_lines(&mut result, &raw, &editor);
    let mut block = Vec::new();
    for (line, kept) in prev.lines.iter().zip(retained) {
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
    Ok(result)
}

/// Reinsert zero-weight blanks and translate candidate indices to physical lines.
/// Blanks do not inherit provenance, but can bridge a physical moved block.
fn restore_physical_lines(result: &mut Change, raw: &[&str], editor: &LineOwner) {
    let mut meaningful = std::mem::take(&mut result.state.lines)
        .into_iter()
        .enumerate();
    let added = std::mem::take(&mut result.added);
    for (position, text) in raw.iter().enumerate() {
        if result.mode.normalize(text).is_empty() {
            result.state.lines.push(Line {
                text: (*text).to_owned(),
                owner: editor.clone(),
            });
            result.added.insert(position, BTreeMap::new());
        } else if let Some((index, line)) = meaningful.next() {
            result.state.lines.push(line);
            if let Some(delta) = added.get(&index) {
                result.added.insert(position, delta.clone());
            }
        }
    }
}

/// Attribute a meaningful replacement hunk and return which old lines retained
/// their owners. Pairing is skipped as a whole when the hunk exceeds its cap.
fn apply_hunk(
    result: &mut Change,
    old: &[&Line],
    new: &[&str],
    editor: &LineOwner,
    policy: Policy,
) -> Result<Vec<bool>, Error> {
    let capped = old
        .len()
        .checked_mul(new.len())
        .is_none_or(|pairs| pairs > MAX_PAIRS);
    result.pairing_capped |= capped;
    let mut used = vec![false; old.len()];
    let mut retained = vec![false; old.len()];
    let characters: Vec<_> = if capped {
        Vec::new()
    } else {
        old.iter()
            .map(|line| Characters::new(result.mode.normalize(&line.text)))
            .collect()
    };
    for text in new {
        let candidate = if capped {
            None
        } else {
            best_match(&characters, &used, result.mode.normalize(text))
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
    Ok(retained)
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

/// Cache Unicode scalars and character frequencies once per candidate line.
struct Characters {
    values: Vec<char>,
    counts: BTreeMap<char, usize>,
}
impl Characters {
    /// Frequency intersection bounds the number of equal characters in any diff.
    fn new(text: &str) -> Self {
        let values: Vec<_> = text.chars().collect();
        let mut counts = BTreeMap::new();
        for &character in &values {
            *counts.entry(character).or_default() += 1;
        }
        Self { values, counts }
    }
}

/// Select the earliest maximum. Safe length/frequency upper bounds skip pairs
/// that cannot beat the current score, without changing Myers ratios or ties.
fn best_match(old: &[Characters], used: &[bool], text: &str) -> Option<(usize, Similarity)> {
    let new = Characters::new(text);
    let mut best: Option<(usize, Similarity)> = None;
    for (index, (line, used)) in old.iter().zip(used).enumerate() {
        if *used {
            continue;
        }
        let denominator = (line.values.len() as u128 + new.values.len() as u128).max(1);
        if let Some((_, current)) = best {
            let length_bound = 2 * line.values.len().min(new.values.len()) as u128;
            if length_bound * current.denominator <= current.numerator * denominator {
                continue;
            }
            let common: usize = new
                .counts
                .iter()
                .map(|(character, count)| (*count).min(*line.counts.get(character).unwrap_or(&0)))
                .sum();
            if 2 * common as u128 * current.denominator <= current.numerator * denominator {
                continue;
            }
        }
        let equal =
            if line.values.len() >= 256 && new.values.len() >= 256 && line.counts.len() <= 128 {
                bit_lcs(&line.values, &new.values)
            } else {
                capture_diff_slices(Algorithm::Myers, &line.values, &new.values)
                    .iter()
                    .filter(|op| op.tag() == DiffTag::Equal)
                    .map(|op| op.old_range().len())
                    .sum()
            };
        let score = Similarity {
            numerator: 2 * equal as u128,
            denominator,
        };
        if best.is_none_or(|(_, current)| {
            score.numerator * current.denominator > current.numerator * score.denominator
        }) {
            best = Some((index, score));
        }
    }
    best
}

/// Exact bit-parallel longest-common-subsequence length for long lines with a
/// bounded alphabet. Myers' equal-character count is the same LCS length. Each
/// bit marks a rise in a DP row; `x & !(x - ((row << 1) | 1))` advances that row.
/// Multiword subtraction propagates borrow, and shifts propagate carry. Callers
/// limit the old alphabet to 128 scalars, bounding mask storage to 16 bytes/scalar.
fn bit_lcs(old: &[char], new: &[char]) -> usize {
    let words = old.len().div_ceil(64);
    let mut masks = BTreeMap::new();
    for (position, &character) in old.iter().enumerate() {
        let mask = masks.entry(character).or_insert_with(|| vec![0_u64; words]);
        if let Some(word) = mask.get_mut(position / 64) {
            *word |= 1 << (position % 64);
        }
    }
    let mut row = vec![0_u64; words];
    for character in new {
        let Some(mask) = masks.get(character) else {
            continue;
        };
        let mut carry = 1_u64;
        let mut borrow = false;
        for (word, matches) in row.iter_mut().zip(mask) {
            let x = *word | matches;
            let y = (*word << 1) | carry;
            carry = *word >> 63;
            let (difference, first_borrow) = x.overflowing_sub(y);
            let (difference, second_borrow) = difference.overflowing_sub(u64::from(borrow));
            borrow = first_borrow || second_borrow;
            *word = x & !difference;
        }
    }
    row.iter().map(|word| word.count_ones() as usize).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify bit arithmetic across word boundaries, empty inputs and Unicode.
    #[test]
    fn bit_parallel_lcs_matches_myers() {
        let mut seed = 0x1234_5678_9abc_def0_u64;
        let mut sample = |length| -> Vec<char> {
            (0..length)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    ['a', 'b', 'c', 'λ', 'γ', ' '][usize::try_from(seed % 6).unwrap()]
                })
                .collect()
        };
        for left in [0, 1, 63, 64, 65, 127, 128, 129, 255, 256, 257, 511] {
            for right in [0, 1, 63, 64, 65, 127, 128, 129, 255, 256, 257, 511] {
                let old = sample(left);
                let new = sample(right);
                let equal: usize = capture_diff_slices(Algorithm::Myers, &old, &new)
                    .iter()
                    .filter(|op| op.tag() == DiffTag::Equal)
                    .map(|op| op.old_range().len())
                    .sum();
                assert_eq!(bit_lcs(&old, &new), equal, "lengths {left}, {right}");
                if left > 0 && right > 0 {
                    let old_text: String = old.iter().collect();
                    let new_text: String = new.iter().collect();
                    let (_, score) =
                        best_match(&[Characters::new(&old_text)], &[false], &new_text).unwrap();
                    assert_eq!(score.numerator, 2 * equal as u128);
                }
            }
        }
    }

    /// Bounds must preserve the exhaustive Myers maximum and earliest-position tie.
    #[test]
    fn bounded_pairing_matches_exhaustive_unicode_pairing() {
        let samples = [
            "a",
            "bb",
            "ab",
            "ba",
            "abc",
            "λγ",
            "λ",
            "zzzzzzzz",
            "prefix one",
            "prefix two",
        ];
        let old: Vec<_> = samples.iter().map(|text| Characters::new(text)).collect();
        for mask in 0..(1_usize << samples.len()) {
            let used: Vec<_> = (0..samples.len()).map(|i| mask & (1 << i) != 0).collect();
            for text in samples {
                let new: Vec<_> = text.chars().collect();
                let mut expected: Option<(usize, Similarity)> = None;
                for (index, (line, used)) in old.iter().zip(&used).enumerate() {
                    if *used {
                        continue;
                    }
                    let equal: usize = capture_diff_slices(Algorithm::Myers, &line.values, &new)
                        .iter()
                        .filter(|op| op.tag() == DiffTag::Equal)
                        .map(|op| op.old_range().len())
                        .sum();
                    let score = Similarity {
                        numerator: 2 * equal as u128,
                        denominator: (line.values.len() + new.len()) as u128,
                    };
                    if expected.is_none_or(|(_, current)| {
                        score.numerator * current.denominator
                            > current.numerator * score.denominator
                    }) {
                        expected = Some((index, score));
                    }
                }
                assert_eq!(
                    best_match(&old, &used, text).map(|(i, score)| (i, score.parts())),
                    expected.map(|(i, score)| (i, score.parts()))
                );
            }
        }
    }
}
