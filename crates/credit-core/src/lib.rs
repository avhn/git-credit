//! Pure attribution engine for `git-credit`.
//!
//! This crate turns in-memory file versions into credit: it normalizes content,
//! diffs successive versions, decides which author keeps each unit of code, and
//! accumulates two separate ledgers:
//!
//! - **Ownership**: who authored the meaningful content that survives at a
//!   given commit. Small edits and formatting changes do not transfer it.
//! - **Labor**: meaningful work each author contributed over all history,
//!   including code that was later rewritten or deleted.
//!
//! The crate performs no I/O, reads no clocks and holds no global state, so
//! every result is a deterministic function of its inputs. Repository access
//! lives in the `credit-git` crate.
#![deny(missing_docs)]

use std::fmt;

/// Version of the attribution algorithm.
///
/// Reports embed this number so results computed by different releases are
/// never silently compared. It changes whenever the same history and policy
/// could produce different credit.
pub const ALGORITHM_VERSION: u32 = 0;

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
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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
