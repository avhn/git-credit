//! TTY-aware presentation, separate from the stable piped and JSON contracts.

use super::{Report, lines, plain, rows};
use clap::ValueEnum;
use std::{fmt::Write, io::IsTerminal};

/// Explicit color policy, with `NO_COLOR` and --no-color taking precedence.
#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub(crate) enum Color {
    /// Use ANSI only when stdout is a terminal.
    #[default]
    Auto,
    /// Emit ANSI even when redirected, unless color is explicitly disabled.
    Always,
    /// Never emit ANSI.
    Never,
}

/// Independent output flags, which never affect attribution or JSON.
#[derive(Debug, Default, clap::Args)]
#[allow(clippy::struct_excessive_bools)] // Independent presentation overrides are clap switches.
pub(crate) struct Terminal {
    /// Disable ANSI color; nonempty `NO_COLOR` has the same effect.
    #[arg(long)]
    no_color: bool,
    /// ANSI policy; `NO_COLOR` and --no-color override always.
    #[arg(long, value_enum, default_value_t = Color::Auto)]
    color: Color,
    /// Use ASCII bars and tree branches instead of Unicode.
    #[arg(long)]
    ascii: bool,
    /// Show an indented directory ownership tree at --depth.
    #[arg(long, conflicts_with = "json")]
    tree: bool,
    /// Show ownership bars even when stdout is redirected.
    #[arg(long, conflicts_with = "no_bars")]
    bars: bool,
    /// Hide ownership bars, including on terminals.
    #[arg(long)]
    no_bars: bool,
}

/// Resolved environment, injectable for deterministic tests without global mutation.
#[derive(Clone, Copy)]
#[allow(clippy::struct_excessive_bools)] // Independent resolved presentation capabilities, not application states.
struct Display {
    color: bool,
    bars: bool,
    ascii: bool,
    columns: usize,
    tty: bool,
}

impl Terminal {
    /// Resolve terminal and locale hints once per render, never from commit data.
    fn display(&self) -> Display {
        let tty = std::io::stdout().is_terminal();
        let disabled = self.no_color || std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
        let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
            .iter()
            .find_map(|key| std::env::var(key).ok().filter(|v| !v.is_empty()));
        let ascii = self.ascii
            || locale.is_some_and(|locale| {
                let lower = locale.to_ascii_lowercase();
                !lower.contains("utf-8") && !lower.contains("utf8")
            });
        Display {
            color: !disabled
                && match self.color {
                    Color::Auto => tty,
                    Color::Always => true,
                    Color::Never => false,
                },
            bars: !self.no_bars && (self.bars || tty),
            ascii,
            tty,
            columns: std::env::var("COLUMNS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(80)
                .clamp(40, 512),
        }
    }
}

/// Apply only trusted ANSI styling around already-sanitized content.
fn styled(value: &str, code: &str, enabled: bool) -> String {
    if enabled {
        format!("\x1b[{code}m{value}\x1b[0m")
    } else {
        value.to_owned()
    }
}

/// Rank colors use the SVG palette's order, with brighter dark-surface counterparts.
fn author_color(value: &str, rank: usize, enabled: bool) -> String {
    let hex = credit_badge::Theme::Dark
        .color(rank)
        .trim_start_matches('#');
    let number = u32::from_str_radix(hex, 16).unwrap_or(0x00ff_ffff);
    styled(
        value,
        &format!(
            "38;2;{};{};{}",
            (number >> 16) & 255,
            (number >> 8) & 255,
            number & 255
        ),
        enabled,
    )
}

/// Select the opt-in tree or enhanced table; plain pipes retain the original bytes.
pub(crate) fn render(report: &Report, options: &Terminal) -> String {
    let display = options.display();
    if options.tree {
        tree(report, display)
    } else {
        let mut out = table(report, display);
        append_activity(&mut out, report, display);
        out
    }
}

/// Add activity only for a TTY table with timeline analysis explicitly enabled.
fn append_activity(out: &mut String, report: &Report, display: Display) {
    if !display.tty {
        return;
    }
    if let Some(timeline) = &report.timeline {
        let weeks: Vec<_> = timeline
            .activity
            .iter()
            .map(|week| (week.week_start.as_str(), week.total))
            .collect();
        if let Ok(sparkline) =
            credit_badge::activity_sparkline(&weeks, display.ascii, display.columns)
        {
            out.push('\n');
            out.push_str(&sparkline);
        }
    }
}

/// Warning diagnostics use the same explicit policy as the result stream.
pub(crate) fn warning(value: &str, options: &Terminal) -> String {
    styled(
        &format!("warning: {}", plain(value)),
        "33",
        options.display().color,
    )
}

/// Add inline proportional cells while preserving all original numeric columns.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // Valid shares and bounded widths yield 0..=40 cells.
fn table(report: &Report, display: Display) -> String {
    if !display.color && !display.bars {
        return super::table(report);
    }
    let original = super::table(report);
    let authors = rows(report);
    let name_width = authors
        .iter()
        .map(|a| plain(a.name).chars().count())
        .max()
        .unwrap_or(6)
        .max(6);
    let cells = display.columns.saturating_sub(name_width + 44).clamp(4, 20);
    let mut out = String::new();
    for (index, line) in original.lines().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        if index == 0 {
            out.push_str(&styled(line, "1", display.color));
            if display.bars {
                out.push_str(&styled("  OWNERSHIP", "1", display.color));
            }
        } else if let Some(author) = authors.get(index - 1) {
            out.push_str(&author_color(line, index - 1, display.color));
            if display.bars {
                let owned = (author.ownership_share * f64::from(u32::try_from(cells).unwrap_or(20)))
                    .round() as usize;
                let (full, empty) = if display.ascii {
                    ("#", ".")
                } else {
                    ("█", "░")
                };
                let bar = format!(
                    "  {}{}",
                    full.repeat(owned.min(cells)),
                    empty.repeat(cells.saturating_sub(owned))
                );
                out.push_str(&author_color(&bar, index - 1, display.color));
            }
        } else {
            out.push_str(&styled(line, "2", display.color));
        }
    }
    out
}

/// Print each parent once and append the three leading owners to populated nodes.
fn tree(report: &Report, display: Display) -> String {
    let mut nodes = std::collections::BTreeMap::new();
    for directory in &report.directories {
        let components: Vec<_> = directory.path.split('/').collect();
        for end in 1..=components.len() {
            nodes.entry(components[..end].join("/")).or_insert(None);
        }
        nodes.insert(directory.path.clone(), Some(directory));
    }
    let ranked = rows(report);
    let mut out = styled("DIRECTORY OWNERSHIP", "1", display.color);
    if nodes.is_empty() {
        out.push_str("\n(no directory data; use --depth 1 or greater)");
    }
    for (path, directory) in nodes {
        let depth = path.split('/').count().saturating_sub(1);
        let branch = if display.ascii { "+- " } else { "└─ " };
        let _ = write!(
            out,
            "\n{}{}{}",
            "  ".repeat(depth),
            branch,
            plain(if path == "." {
                "(root)"
            } else {
                path.rsplit('/').next().unwrap_or(&path)
            })
        );
        if let Some(directory) = directory {
            let mut owners: Vec<_> = directory
                .authors
                .iter()
                .filter_map(|(id, amount)| report.authors.get(id.0 as usize).map(|a| (a, *amount)))
                .collect();
            owners.sort_by(|a, b| {
                b.1.cmp(&a.1)
                    .then_with(|| a.0.name.cmp(&b.0.name))
                    .then_with(|| a.0.email.cmp(&b.0.email))
            });
            for (index, (owner, amount)) in owners.iter().take(3).enumerate() {
                let percent = if directory.total == 0 {
                    0.0
                } else {
                    lines(*amount) / lines(directory.total) * 100.0
                };
                let rank = ranked
                    .iter()
                    .position(|a| a.email == owner.email)
                    .unwrap_or(7);
                let label = format!("{} {:.1}%", plain(&owner.name), percent);
                out.push_str(if index == 0 { "  " } else { ", " });
                out.push_str(&author_color(&label, rank, display.color));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pure styling removes no data and terminal sanitization neutralizes hostile names.
    #[test]
    fn styles_and_controls_are_deterministic() {
        let hostile = "<script> & \"name\"\x1b[31m\n\u{202e}end";
        let clean = plain(hostile);
        assert!(!clean.chars().any(char::is_control));
        assert!(!clean.contains('\u{202e}'));
        assert_eq!(author_color(&clean, 0, true), author_color(&clean, 0, true));
        assert_eq!(styled(&clean, "1", false), clean);
    }

    /// Inject TTY capabilities so sparkline gating does not depend on the test runner.
    #[test]
    fn activity_requires_timeline_and_a_tty() {
        let mut report = Report {
            timeline: None,
            cache_diagnostics: Vec::new(),
            granularity: credit_git::Granularity::Token,
            bulk_edit_commits: 0,
            walk: credit_git::Walk::Full,
            peak_retained_states: 0,
            head: "a".repeat(40),
            commits_analyzed: 1,
            formatting_commits: 0,
            authors: Vec::new(),
            ledger: credit_core::Ledger::default(),
            directories: Vec::new(),
            warnings: Vec::new(),
        };
        let mut display = Display {
            color: false,
            bars: false,
            ascii: true,
            columns: 80,
            tty: true,
        };
        let mut out = "table".to_owned();
        append_activity(&mut out, &report, display);
        assert_eq!(out, "table");
        report.timeline = Some(credit_git::Timeline {
            activity: vec![credit_git::ActivityBucket {
                week_start: "2020-01-06T00:00:00+00:00".into(),
                commits: 1,
                total: 2_000_000,
                authors: credit_core::LaborDelta::default(),
            }],
            ownership: Vec::new(),
        });
        display.tty = false;
        append_activity(&mut out, &report, display);
        assert_eq!(out, "table");
        display.tty = true;
        append_activity(&mut out, &report, display);
        assert!(out.starts_with("table\nActivity"));
        assert!(out.ends_with('@'));
        assert!(out.is_ascii());
        display.ascii = false;
        out.clear();
        append_activity(&mut out, &report, display);
        assert!(out.ends_with('█'));
    }
}
