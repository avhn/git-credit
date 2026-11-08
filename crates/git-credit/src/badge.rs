//! Offline badge orchestration, schema import and generated-file ownership.

use crate::{Analyze, config::Settings, output};
use anyhow::{Context, bail};
use clap::ValueEnum;
use credit_badge::{Author, Options, Report};
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Badge assets selected for this invocation; endpoint/report JSON is always written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
enum Style {
    /// Stacked contributor ownership bar and legend.
    Bar,
    /// Ownership/labor summary card.
    Card,
    /// Strict-majority bus-factor flat badge.
    BusFactor,
    /// One safely named flat badge per top contributor.
    Authors,
    /// Every supported SVG style.
    #[default]
    All,
}

/// Summary-card color scheme.
#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum Theme {
    /// White background and dark text.
    #[default]
    Light,
    /// Dark background and light text.
    Dark,
}

/// Badge command arguments; analysis policy comes from the same root config as analyze.
#[derive(Debug, clap::Args)]
pub(crate) struct Badge {
    /// Disable reading and writing the incremental replay cache.
    #[arg(long)]
    no_cache: bool,
    /// Override the cache directory (default: inside the repository Git directory).
    #[arg(long)]
    cache_dir: Option<PathBuf>,
    /// Retained first-parent ancestor snapshots in addition to HEAD.
    #[arg(long, default_value_t = 64)]
    cache_depth: usize,
    /// Repository path; ignored when importing a report with --from.
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Generated asset directory; existing generated assets are replaced.
    #[arg(long)]
    out_dir: PathBuf,
    /// SVG styles to generate; endpoint and report JSON are always included.
    #[arg(long, value_enum, default_value_t = Style::All)]
    style: Style,
    /// Maximum named contributors, with the rest grouped as others.
    #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u32).range(1..=1000))]
    top: u32,
    /// Summary-card theme.
    #[arg(long, value_enum, default_value_t = Theme::Light)]
    theme: Theme,
    /// Bar and card width in pixels.
    #[arg(long, default_value_t = 600, value_parser = clap::value_parser!(u32).range(200..=4096))]
    width: u32,
    /// Import an already privacy-filtered schema-version-1 analyze JSON report.
    #[arg(long = "from", value_name = "REPORT_JSON")]
    from: Option<PathBuf>,
    /// Print written paths and analysis warnings to stderr; stdout remains empty.
    #[arg(short = 'v', action = clap::ArgAction::Count)]
    verbose: u8,
}

/// The required rendering subset of the public JSON contract; other fields survive
/// unchanged in credit.json so importing does not discard policy or directory data.
#[derive(Deserialize)]
struct Imported {
    schema_version: u32,
    units: String,
    head: String,
    commits_analyzed: u64,
    authors: Vec<ImportedAuthor>,
}

/// Numeric presentation fields; identities have already been filtered by analysis.
#[derive(Deserialize)]
struct ImportedAuthor {
    name: String,
    ownership_share: f64,
    labor: f64,
}

/// Analyze through the existing validated boundary or import JSON, then render
/// everything before touching output files. No output bytes are written to stdout.
pub(crate) fn run(args: &Badge) -> anyhow::Result<()> {
    let source = if let Some(path) = &args.from {
        fs::read_to_string(path)
            .with_context(|| format!("cannot read report {}", path.display()))?
    } else {
        let analysis = Analyze {
            no_cache: args.no_cache,
            cache_dir: args.cache_dir.clone(),
            cache_depth: args.cache_depth,
            path: args.path.clone(),
            first_parent: false,
            json: true,
            by_email: false,
            keep: None,
            granularity: None,
            no_bulk_discount: false,
            exclude: Vec::new(),
            depth: 1,
            no_color: true,
            quiet: args.verbose == 0,
            verbose: args.verbose,
        };
        let settings = Settings::load(&analysis)?;
        let report = credit_git::analyze(&analysis.path, &settings.options, &|path| {
            settings.excluded.is_match(path)
        })?;
        if args.verbose > 0 {
            for diagnostic in &report.cache_diagnostics {
                eprintln!("{}", output::plain(diagnostic));
            }
            for warning in &report.warnings {
                eprintln!("warning: {}", output::plain(warning));
            }
        }
        output::json(&report, &settings)?
    };
    let report = import(&source)?;
    let assets = credit_badge::render(
        &report,
        Options {
            top: usize::try_from(args.top)?,
            width: args.width,
            theme: match args.theme {
                Theme::Light => credit_badge::Theme::Light,
                Theme::Dark => credit_badge::Theme::Dark,
            },
        },
    )?;
    let mut files = vec![
        (
            PathBuf::from("credit.json"),
            format!("{}\n", source.trim_end()),
        ),
        (PathBuf::from("bus-factor.json"), assets.bus_factor_json),
        (
            PathBuf::from("top-contributor.json"),
            assets.top_contributor_json,
        ),
    ];
    if matches!(args.style, Style::Bar | Style::All) {
        files.push(("credit-bar.svg".into(), assets.bar));
    }
    if matches!(args.style, Style::Card | Style::All) {
        files.push(("credit-card.svg".into(), assets.card));
    }
    if matches!(args.style, Style::BusFactor | Style::All) {
        files.push(("bus-factor.svg".into(), assets.bus_factor));
    }
    if matches!(args.style, Style::Authors | Style::All) {
        files.extend(
            assets
                .authors
                .into_iter()
                .map(|(stem, svg)| (PathBuf::from("authors").join(format!("{stem}.svg")), svg)),
        );
    }
    write_files(&args.out_dir, &files, args.verbose > 0)
}

/// Reject incompatible reports before producing any file, rather than guessing units.
fn import(source: &str) -> anyhow::Result<Report> {
    let report: Imported = serde_json::from_str(source).context("invalid analyze JSON report")?;
    if report.schema_version != 1 {
        bail!(
            "unsupported report schema_version {}; expected 1",
            report.schema_version
        );
    }
    if report.units != "lines" {
        bail!("unsupported report units {}; expected lines", report.units);
    }
    if report.head.len() != 40 || !report.head.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("invalid report HEAD; expected a full 40-character SHA-1 object ID");
    }
    Ok(Report {
        authors: report
            .authors
            .into_iter()
            .map(|a| Author {
                name: a.name,
                ownership_share: a.ownership_share,
                labor: a.labor,
            })
            .collect(),
        commits: report.commits_analyzed,
        head: report.head,
    })
}

/// Refuse symlinked output components to avoid following an author-controlled path.
///
/// A relative path is resolved inside the checkout, where a committed symlink could
/// redirect writes, so every component is checked. An absolute path was chosen by
/// the caller, and system symlinks above it (for example macOS `/var` pointing to
/// `/private/var`) are legitimate, so only the path itself is checked.
fn check_path(path: &Path) -> anyhow::Result<()> {
    let checked: Vec<&Path> = if path.is_absolute() {
        vec![path]
    } else {
        path.ancestors()
            .filter(|ancestor| !ancestor.as_os_str().is_empty())
            .collect()
    };
    for ancestor in checked {
        match fs::symlink_metadata(ancestor) {
            Ok(meta) if meta.file_type().is_symlink() => {
                bail!("refusing symlinked badge output {}", ancestor.display())
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).context("cannot inspect badge output path"),
        }
    }
    Ok(())
}

/// Replace only our named assets, removing stale generated SVGs so renamed or
/// newly opted-out identities cannot linger. Unrelated files are never deleted.
fn write_files(out: &Path, files: &[(PathBuf, String)], verbose: bool) -> anyhow::Result<()> {
    check_path(out)?;
    let authors = out.join("authors");
    check_path(&authors)?;
    let mut stale: Vec<_> = ["credit-bar.svg", "credit-card.svg", "bus-factor.svg"]
        .iter()
        .map(|name| out.join(name))
        .collect();
    if authors.is_dir() {
        for entry in fs::read_dir(&authors)? {
            let path = entry?.path();
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("author-"))
                && path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("svg"))
            {
                stale.push(path);
            }
        }
    }
    for path in stale.iter().chain(
        files
            .iter()
            .map(|(path, _)| out.join(path))
            .collect::<Vec<_>>()
            .iter(),
    ) {
        check_path(path)?;
    }
    fs::create_dir_all(out).context("cannot create badge output directory")?;
    for path in stale {
        if !files.iter().any(|(name, _)| out.join(name) == path) && path.is_file() {
            fs::remove_file(path)?;
        }
    }
    for (relative, text) in files {
        let path = out.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, text).with_context(|| format!("cannot write {}", path.display()))?;
        if verbose {
            eprintln!("{}", output::plain(&path.display().to_string()));
        }
    }
    Ok(())
}
