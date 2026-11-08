//! `git-credit`: fair git authorship stats.
//!
//! Reports per-author ownership and labor for a repository without letting
//! formatting changes or trivial edits take credit away from the original
//! author.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// Fair git authorship stats: ownership and labor per author that formatting
/// and trivial edits cannot steal.
#[derive(Debug, Parser)]
#[command(name = "git-credit", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Analyze a repository and report ownership and labor per author.
    Analyze {
        /// Path to the repository to analyze.
        #[arg(default_value = ".")]
        repo: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Analyze { repo } => {
            eprintln!(
                "git-credit: `analyze` is not implemented yet (algorithm version {}); \
                 requested repository: {}",
                credit_git::ALGORITHM_VERSION,
                repo.display()
            );
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::Cli;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
