//! Repository access for `git-credit`.
//!
//! This crate reads a Git repository and feeds file versions, in history
//! order, to the pure engine in `credit-core`. It owns everything that touches
//! the repository: walking commits, following renames, resolving `.mailmap`
//! identities and parsing `Co-authored-by` trailers.
#![deny(missing_docs)]

pub use credit_core::ALGORITHM_VERSION;
