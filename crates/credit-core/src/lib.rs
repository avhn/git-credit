//! Pure attribution engine for `git-credit`.
//!
//! This crate turns in-memory file versions into credit: it normalizes content,
//! diffs successive versions, decides which author keeps each unit of code, and
//! accumulates two separate ledgers:
//!
//! - **Ownership**: who authored the meaningful content that survives at a
//!   given commit. Unchanged tokens and formatting changes retain provenance.
//! - **Labor**: meaningful work each author contributed over all history,
//!   including code that was later rewritten or deleted.
//!
//! The crate performs no I/O, reads no clocks and holds no global state, so
//! every result is a deterministic function of its inputs. Repository access
//! lives in the `credit-git` crate.
//!
//! Ownership and labor use millionths of a meaningful line. Token mode splits
//! each line by lexical weights; the optional legacy line engine uses character
//! similarity. Callers reconcile parent provenance, same-commit moves and bulk
//! edits before accumulating labor, then recount live ownership after deletions.
//!
//! ```
//! use credit_core::{AuthorId, FileState, Ledger, LineOwner, NormalizationMode,
//!     Policy, apply_change};
//! let authors = LineOwner::equal(&[AuthorId(0), AuthorId(1)])?;
//! let change = apply_change(&FileState::default(), "hello\n", authors.authors(),
//!     NormalizationMode::IgnoreIndentation, Policy::default())?;
//! let mut ledger = Ledger::default();
//! ledger.add_labor(&change.labor)?;
//! ledger.recount([&change.state])?;
//! assert_eq!(ledger.ownership[&AuthorId(0)], 500_000);
//! # Ok::<(), credit_core::Error>(())
//! ```
#![deny(missing_docs)]

use std::fmt;

/// Version of the attribution algorithm.
///
/// Reports embed this number so results computed by different releases are
/// never silently compared. It changes whenever the same history and policy
/// could produce different credit.
pub const ALGORITHM_VERSION: u32 = 4;

mod engine;
mod lexer;
mod model;
mod moves;
mod parents;
mod token_engine;

pub use engine::{Change, ChangeKind, apply_change};
pub use lexer::{PUNCTUATION_WEIGHT, TOKEN_WEIGHT, Token, TokenKind, tokenize};
pub use model::{
    AuthorId, FileState, LaborDelta, Ledger, Line, LineOwner, NormalizationMode, Policy, SCALE,
};
pub use moves::{reconcile_merge_moves, reconcile_moves};
pub use parents::{adopt_parent, reconcile_parents};
pub use token_engine::{
    OwnedToken, TokenChange, TokenState, adopt_token_parent, apply_token_change,
    discount_bulk_edits, reconcile_token_moves, reconcile_token_parents,
};

/// Errors produced by the attribution engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A configured threshold lies outside its valid range.
    InvalidThreshold {
        /// Name of the offending setting.
        name: &'static str,
        /// The rejected value, in parts per thousand.
        per_mille: u32,
    },
    /// Author shares must be nonempty, unique, positive, and sum to one line.
    InvalidShares,
    /// A fixed-point ledger cannot represent the requested total.
    Overflow,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidShares => write!(
                f,
                "author shares must be unique, positive, and sum to 1000000"
            ),
            Self::Overflow => write!(f, "fixed-point credit total overflow"),
            Self::InvalidThreshold { name, per_mille } => {
                write!(
                    f,
                    "threshold `{name}` must be at most 1000 per mille, got {per_mille}"
                )
            }
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
#[path = "tests.rs"]
mod attribution_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_message_names_the_setting() {
        let err = Error::InvalidThreshold {
            name: "similarity",
            per_mille: 1200,
        };
        assert_eq!(
            err.to_string(),
            "threshold `similarity` must be at most 1000 per mille, got 1200"
        );
    }
}
