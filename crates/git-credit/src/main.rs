//! Offline ownership and labor reports for committed Git history.

mod config;
mod output;

use clap::{Parser, Subcommand};
use std::{io::Write, path::PathBuf, process::ExitCode};

/// Fair git authorship stats. Provenance metrics do not measure a person's value.
#[derive(Debug, Parser)]
#[command(name = "git-credit", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
/// Supported offline operations.
#[derive(Debug, Subcommand)]
enum Command {
    /// Analyze committed HEAD using the first-parent history chain.
    Analyze(Analyze),
}
/// Analysis inputs and output controls. CLI policy values override configuration.
#[derive(Debug, clap::Args)]
#[allow(clippy::struct_excessive_bools)] // Independent clap switches, not mutually exclusive application states.
struct Analyze {
    /// Repository path (working tree or bare repository).
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Emit schema-versioned JSON instead of a table (see docs/json.md).
    #[arg(long)]
    json: bool,
    /// Group by canonical email instead of case-insensitive resolved display name.
    #[arg(long)]
    by_email: bool,
    /// Similarity required to keep ownership, 0.0..=1.0 [default: 0.70].
    #[arg(long, value_parser = config::parse_keep)]
    keep: Option<u32>,
    /// Exclude a repository-relative glob; repeat to replace config exclusions.
    #[arg(long, value_parser = config::parse_glob)]
    exclude: Vec<String>,
    /// Directory breakdown depth; zero disables it.
    #[arg(long, default_value_t = 1)]
    depth: usize,
    /// Disable color (output is currently always plain text; `NO_COLOR` is honored).
    #[arg(long)]
    no_color: bool,
    /// Suppress nonessential diagnostics, not results or errors.
    #[arg(long, short = 'q', conflicts_with = "verbose")]
    quiet: bool,
    /// Print analysis diagnostics to stderr; repeat for increased verbosity.
    #[arg(short = 'v', action = clap::ArgAction::Count)]
    verbose: u8,
}

/// Parse usage errors as exit 2, analysis errors as exit 1, successful output as 0.
fn main() -> ExitCode {
    let Cli {
        command: Command::Analyze(args),
    } = Cli::parse();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("git-credit: {}", output::plain(&format!("{error:#}")));
            ExitCode::from(1)
        }
    }
}

/// Resolve policy at the config boundary and keep diagnostics off stdout.
fn run(args: &Analyze) -> anyhow::Result<()> {
    let settings = config::Settings::load(args)?;
    if args.verbose > 0 {
        eprintln!("Analyzing committed HEAD (first-parent, offline)...");
    }
    let report = credit_git::analyze(&args.path, &settings.options, &|path| {
        settings.excluded.is_match(path)
    })?;
    if !args.quiet {
        for warning in &report.warnings {
            eprintln!("warning: {}", output::plain(warning));
        }
        if args.verbose > 0 {
            eprintln!(
                "Analyzed {} commits; {} identities",
                report.commits_analyzed,
                report.authors.len()
            );
        }
    }
    let rendered = if args.json {
        output::json(&report, &settings)?
    } else {
        output::table(&report)
    };
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(rendered.as_bytes())?;
    stdout.write_all(b"\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;
    #[test]
    fn cli_definition_is_valid() {
        super::Cli::command().debug_assert();
    }
}
