//! Offline first-parent repository replay for line-level attribution.
//!
//! Only live file attribution is retained. Commit IDs are collected to reverse
//! the first-parent chain; side branches are not replayed. Merge diffs therefore
//! credit the merge author and trailers, an explicitly approximate walk.
//! ```no_run
//! let report = credit_git::analyze(std::path::Path::new("."),
//!     &credit_git::Options::default(), &|_| false)?;
//! assert!(report.commits_analyzed > 0);
//! # Ok::<(), credit_git::Error>(())
//! ```
#![deny(missing_docs)]

mod identities;
mod replay;

pub use credit_core::ALGORITHM_VERSION;
use credit_core::{AuthorId, Policy};
pub use replay::analyze;
use std::{
    collections::BTreeMap,
    fmt,
    path::{Path, PathBuf},
};

/// Repository analysis failures. No network recovery is attempted.
#[derive(Debug)]
pub enum Error {
    /// The repository has incomplete history.
    ShallowRepository,
    /// HEAD has not yet been created.
    EmptyRepository,
    /// An object, reference, configuration or repository operation failed.
    Repository(String),
    /// An input file could not be read.
    Io(std::io::Error),
    /// Core policy or accounting validation failed.
    Core(credit_core::Error),
    /// A path or identity cannot be represented without ambiguity.
    InvalidInput(String),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ShallowRepository => write!(
                f,
                "shallow repository: run `git fetch --unshallow` and retry"
            ),
            Self::EmptyRepository => write!(
                f,
                "empty repository: create a commit before running analyze"
            ),
            Self::Repository(e) => write!(
                f,
                "repository analysis failed: {e}; ensure all history objects are available"
            ),
            Self::Io(e) => write!(f, "cannot read analysis input: {e}"),
            Self::Core(e) => e.fmt(f),
            Self::InvalidInput(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for Error {}
impl From<credit_core::Error> for Error {
    fn from(e: credit_core::Error) -> Self {
        Self::Core(e)
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
/// Preserve a repository diagnostic without using application error types.
fn git_error(e: impl fmt::Display) -> Error {
    Error::Repository(e.to_string())
}

/// Identity grouping applied after resolving committed mailmap identities.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GroupBy {
    /// Merge case-insensitive trimmed display names; empty names fall back to email.
    #[default]
    Name,
    /// Keep distinct canonical emails separate, regardless of display names.
    Email,
}

/// Analysis policy, independent of CLI serialization and glob implementation.
#[derive(Debug, Clone)]
pub struct Options {
    /// Attribution threshold.
    pub policy: Policy,
    /// Maximum blob size in bytes, default 1 MiB.
    pub max_file_size: u64,
    /// Directory component depth; zero disables directory reporting.
    pub depth: usize,
    /// Canonical emails to aggregate under the anonymous identity.
    pub opt_out: Vec<String>,
    /// Group by resolved display name by default, or canonical email explicitly.
    pub group_by: GroupBy,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            policy: Policy::default(),
            max_file_size: 1_048_576,
            depth: 1,
            opt_out: Vec::new(),
            group_by: GroupBy::default(),
        }
    }
}

/// Canonical display identity, resolved consistently using HEAD's mailmap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Author {
    /// Display name, or `anonymous` for opted-out identities.
    pub name: String,
    /// Most frequently used canonical email; lexical tie-break. Empty for anonymous.
    pub email: String,
    /// All merged canonical emails in sorted order. Empty for anonymous.
    pub emails: Vec<String>,
}
/// Ownership by author within a directory, in millionths of a nonblank line.
#[derive(Debug, Clone)]
pub struct Directory {
    /// Slash-separated directory path, or `.` for root files.
    pub path: String,
    /// Sum of author ownership in this directory.
    pub total: u64,
    /// Attribution indexed by the report's authors table.
    pub authors: BTreeMap<AuthorId, u64>,
}
/// Deterministic analysis output. All credit amounts are millionths of a line.
#[derive(Debug)]
pub struct Report {
    /// Full analyzed HEAD object ID.
    pub head: String,
    /// Number of visited first-parent commits.
    pub commits_analyzed: u64,
    /// Commits whose included normalized nonblank content is unchanged.
    pub formatting_commits: u64,
    /// Identity table indexed by `AuthorId`.
    pub authors: Vec<Author>,
    /// Current ownership and historical labor.
    pub ledger: credit_core::Ledger,
    /// Directory ownership, sorted by path.
    pub directories: Vec<Directory>,
    /// Stable, deduplicated diagnostics, without opted-out identities.
    pub warnings: Vec<String>,
}

/// Discover the repository's working root (or Git directory for a bare repo).
///
/// # Errors
/// Returns a repository diagnostic when discovery fails.
pub fn repository_root(path: &Path) -> Result<PathBuf, Error> {
    let repo = gix::discover(path).map_err(git_error)?;
    Ok(repo
        .workdir()
        .unwrap_or_else(|| repo.git_dir())
        .to_path_buf())
}

/// Select conservative indentation semantics from a repository-relative path.
/// Known non-indentation-sensitive languages ignore leading whitespace; every
/// unknown extension and extensionless file remains indentation-sensitive.
pub fn normalization(path: &str) -> credit_core::NormalizationMode {
    use credit_core::NormalizationMode::{IgnoreIndentation, IndentationSignificant};
    match Path::new(path).extension().and_then(|s| s.to_str()) {
        Some(
            "rs" | "js" | "jsx" | "ts" | "tsx" | "c" | "h" | "cpp" | "hpp" | "cc" | "cs" | "java"
            | "go" | "php" | "rb" | "swift" | "kt" | "kts" | "json" | "toml" | "css" | "scss"
            | "html" | "xml" | "sql" | "sh" | "bash" | "zsh",
        ) => IgnoreIndentation,
        _ => IndentationSignificant,
    }
}
