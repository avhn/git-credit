//! Deterministic versioned JSON and terminal-safe plain text rendering.

use crate::config::Settings;
use credit_core::AuthorId;
use credit_git::Report;
use serde::Serialize;
use std::fmt::Write;

/// Convert fixed-point totals only at the presentation boundary. Very large
/// totals may round in JSON's floating representation; internal accounting is exact.
#[allow(clippy::cast_precision_loss)] // Display precision, never attribution arithmetic.
fn lines(parts: u64) -> f64 {
    parts as f64 / f64::from(credit_core::SCALE)
}

/// Remove terminal control characters from untrusted author/path diagnostics.
pub(crate) fn plain(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// Numeric author summary shared by both renderers.
#[derive(Serialize)]
struct AuthorRow<'a> {
    name: &'a str,
    email: &'a str,
    emails: &'a [String],
    ownership_lines: f64,
    ownership_share: f64,
    labor: f64,
}
/// Directory-specific author share; shares are fractions in 0..=1.
#[derive(Serialize)]
struct DirectoryAuthor<'a> {
    email: &'a str,
    lines: f64,
    share: f64,
}
/// Directory ownership at the requested depth.
#[derive(Serialize)]
struct DirectoryRow<'a> {
    path: &'a str,
    total_lines: f64,
    authors: Vec<DirectoryAuthor<'a>>,
}
/// Effective scoring policy, including built-in exclusions.
#[derive(Serialize)]
struct PolicyRow<'a> {
    bulk_discount: bool,
    keep: f64,
    group_by: &'static str,
    exclude: &'a [String],
    max_file_size: u64,
}
/// Stable schema version 1; values never include a wall-clock timestamp.
#[derive(Serialize)]
struct JsonReport<'a> {
    schema_version: u32,
    algorithm_version: u32,
    head: &'a str,
    walk: &'static str,
    units: &'static str,
    granularity: &'static str,
    bulk_edit_commits: u64,
    policy: PolicyRow<'a>,
    commits_analyzed: u64,
    formatting_commits: u64,
    authors: Vec<AuthorRow<'a>>,
    directories: Vec<DirectoryRow<'a>>,
    warnings: &'a [String],
}

/// Build rows sorted by exact ownership, then display name and email.
fn rows(report: &Report) -> Vec<AuthorRow<'_>> {
    let total: f64 = report.ledger.ownership.values().map(|v| lines(*v)).sum();
    let mut indexed: Vec<_> = report
        .authors
        .iter()
        .enumerate()
        .filter_map(|(i, author)| {
            let id = AuthorId(u32::try_from(i).ok()?);
            let owned = *report.ledger.ownership.get(&id).unwrap_or(&0);
            let labor = *report.ledger.labor.get(&id).unwrap_or(&0);
            // Formatting-only identities with no surviving work or labor add no data.
            (owned != 0 || labor != 0).then_some((author, owned, labor))
        })
        .collect();
    indexed.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| a.0.name.cmp(&b.0.name))
            .then_with(|| a.0.email.cmp(&b.0.email))
    });
    indexed
        .into_iter()
        .map(|(author, owned, labor)| AuthorRow {
            name: &author.name,
            email: &author.email,
            emails: &author.emails,
            ownership_lines: lines(owned),
            ownership_share: if total == 0.0 {
                0.0
            } else {
                lines(owned) / total
            },
            labor: lines(labor),
        })
        .collect()
}

/// Serialize the public report contract using serde; strings remain JSON escaped.
pub(crate) fn json(report: &Report, settings: &Settings) -> anyhow::Result<String> {
    let directories = report
        .directories
        .iter()
        .map(|directory| {
            let total = lines(directory.total);
            let authors = directory
                .authors
                .iter()
                .filter_map(|(id, amount)| {
                    report
                        .authors
                        .get(id.0 as usize)
                        .map(|author| DirectoryAuthor {
                            email: &author.email,
                            lines: lines(*amount),
                            share: if total == 0.0 {
                                0.0
                            } else {
                                lines(*amount) / total
                            },
                        })
                })
                .collect();
            DirectoryRow {
                path: &directory.path,
                total_lines: total,
                authors,
            }
        })
        .collect();
    Ok(serde_json::to_string_pretty(&JsonReport {
        schema_version: 1,
        algorithm_version: credit_core::ALGORITHM_VERSION,
        head: &report.head,
        walk: report.walk.as_str(),
        units: "lines",
        granularity: report.granularity.as_str(),
        bulk_edit_commits: report.bulk_edit_commits,
        policy: PolicyRow {
            bulk_discount: settings.options.bulk_discount
                && report.granularity == credit_git::Granularity::Token,
            keep: f64::from(settings.options.policy.keep_per_mille) / 1000.0,
            exclude: &settings.patterns,
            max_file_size: settings.options.max_file_size,
            group_by: match settings.options.group_by {
                credit_git::GroupBy::Name => "name",
                credit_git::GroupBy::Email => "email",
            },
        },
        commits_analyzed: report.commits_analyzed,
        formatting_commits: report.formatting_commits,
        authors: rows(report),
        directories,
        warnings: &report.warnings,
    })?)
}

/// Render aligned text without ANSI sequences, regardless of TTY or color flags.
pub(crate) fn table(report: &Report) -> String {
    let authors = rows(report);
    let width = authors
        .iter()
        .map(|row| plain(row.name).chars().count())
        .max()
        .unwrap_or(6)
        .max(6);
    let mut text = format!("{:<width$}  OWNERSHIP %  OWNED LINES  LABOR", "AUTHOR");
    for author in authors {
        let _ = write!(
            text,
            "\n{:<width$}  {:>10.1}%  {:>11.1}  {:.1}",
            plain(author.name),
            author.ownership_share * 100.0,
            author.ownership_lines,
            author.labor
        );
    }
    let _ = write!(
        text,
        "\nHEAD {} | {} commits | {} | {} | algorithm {}\nProvenance metrics do not measure a person's value.",
        report.head.get(..12).unwrap_or(&report.head),
        report.commits_analyzed,
        report.walk.as_str(),
        report.granularity.as_str(),
        credit_core::ALGORITHM_VERSION
    );
    text
}
