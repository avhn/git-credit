//! Pure, deterministic SVG 1.1 and shields.io endpoint rendering.
//!
//! Shares are fractions, not percentages. Text uses conservative Verdana 11px
//! width estimates; non-ASCII scalars reserve 12px. Bar and card names are fitted
//! to their columns, with full names retained in XML-escaped titles.
//!
//! ```
//! use credit_badge::{Options, Report, render};
//! let badges = render(&Report::default(), Options::default())?;
//! assert!(badges.bus_factor.contains("no data"));
//! # Ok::<(), credit_badge::Error>(())
//! ```

#![deny(missing_docs)]

use std::fmt::{self, Write};

mod charts;
mod timeline;
mod treemap;

pub use timeline::{Activity, Amount, Ownership, Timeline, activity_sparkline};

/// One already privacy-filtered identity. The renderer never reads identities itself.
#[derive(Clone, Debug, PartialEq)]
pub struct Author {
    /// Public display name, which is XML/JSON escaped on output.
    pub name: String,
    /// Fraction of live ownership in 0..=1.
    pub ownership_share: f64,
    /// Nonnegative finite historical labor in report units.
    pub labor: f64,
}

/// Minimal presentation input, independent of Git and the analysis engine.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    /// Authors; rendering sorts by descending ownership, then name and labor.
    pub authors: Vec<Author>,
    /// Number of analyzed commits, including formatting commits.
    pub commits: u64,
    /// Analyzed HEAD identifier; the card displays its first twelve characters.
    pub head: String,
    /// Total surviving ownership in line-equivalent units.
    pub total_lines: f64,
    /// Flat directory totals at the requested analysis depth.
    pub directories: Vec<Directory>,
    /// Optional historical series, indexed into this report's authors array.
    pub timeline: Option<Timeline>,
}

/// One privacy-filtered directory, colored by its leading owner.
#[derive(Clone, Debug, PartialEq)]
pub struct Directory {
    /// Repository-relative path.
    pub path: String,
    /// Nonnegative finite surviving ownership in lines.
    pub total_lines: f64,
    /// Leading owner's public display name.
    pub owner: String,
    /// Leading owner's fraction of this directory in 0..=1.
    pub owner_share: f64,
}

/// Summary card color scheme.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Theme {
    /// White background and dark text.
    #[default]
    Light,
    /// Dark background and light text.
    Dark,
}

/// Rendering controls, with bounded dimensions to avoid accidental huge output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// Maximum named authors, in 1..=1000; the bar groups the rest as others.
    pub top: usize,
    /// Bar and card width in pixels, in 200..=4096.
    pub width: u32,
    /// Card theme; flat badges use their semantic colors in either theme.
    pub theme: Theme,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            top: 5,
            width: 600,
            theme: Theme::Light,
        }
    }
}

/// All rendered assets; callers choose which ones to write, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Badges {
    /// Stacked ownership bar and legend.
    pub bar: String,
    /// Ownership and labor summary card.
    pub card: String,
    /// Proportional ownership ring and apportioned legend.
    pub donut: String,
    /// Grouped ownership and historical labor bars in line units.
    pub labor: String,
    /// Squarified flat directory ownership map.
    pub treemap: String,
    /// Weekly labor columns, aggregated by month for long spans.
    pub activity: String,
    /// Absolute ownership along the main line of history.
    pub history: String,
    /// Flat bus-factor badge.
    pub bus_factor: String,
    /// Safe unique filename stems paired with per-author SVG strings.
    pub authors: Vec<(String, String)>,
    /// Shields.io bus-factor endpoint JSON.
    pub bus_factor_json: String,
    /// Shields.io top-contributor endpoint JSON.
    pub top_contributor_json: String,
}

/// Invalid presentation inputs; no I/O or attribution failures occur here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Top must be between one and one thousand.
    InvalidTop,
    /// Width must be between 200 and 4096 pixels.
    InvalidWidth,
    /// A share is nonfinite/out of range, or labor is nonfinite/negative.
    InvalidAuthor,
    /// Positive shares must sum to one within one millionth.
    InvalidShareTotal,
    /// A report or directory total/share is not finite or is out of range.
    InvalidDirectory,
    /// Timeline dates, ordering, totals or author references are invalid.
    InvalidTimeline,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidTimeline => "timeline dates, ordering, totals or author references are invalid",
            Self::InvalidDirectory => "report and directory totals must be finite and nonnegative, and directory shares in 0..=1",
            Self::InvalidTop => "top must be in 1..=1000",
            Self::InvalidWidth => "width must be in 200..=4096",
            Self::InvalidAuthor => {
                "author shares must be finite in 0..=1 and labor finite and nonnegative"
            }
            Self::InvalidShareTotal => "positive ownership shares must sum to one within 0.000001",
        })
    }
}
impl std::error::Error for Error {}

/// Render every asset without reading files, clocks, environment or global state.
///
/// Zero ownership (including historical-labor-only reports) yields no-data ownership
/// badges. Cards still show historical labor. Input order does not affect output.
///
/// # Errors
/// Returns a typed error for invalid shares, labor, top count or width.
pub fn render(report: &Report, options: Options) -> Result<Badges, Error> {
    validate(report, options)?;
    let mut authors: Vec<_> = report.authors.iter().collect();
    authors.sort_by(|a, b| {
        b.ownership_share
            .total_cmp(&a.ownership_share)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| b.labor.total_cmp(&a.labor))
    });
    let total: f64 = authors.iter().map(|a| a.ownership_share).sum();
    let factor = bus_factor(&authors, total);
    let (message, color) = match factor {
        0 => ("no data".to_owned(), "#555"),
        1 => ("1".to_owned(), "#c62828"),
        2 => ("2".to_owned(), "#9c6500"),
        n => (n.to_string(), "#237a3b"),
    };
    let mut used = std::collections::BTreeSet::new();
    let author_badges = authors
        .iter()
        .take(options.top)
        .map(|a| {
            let base = slug(&a.name);
            let mut stem = base.clone();
            let mut suffix = 2_u64;
            while !used.insert(stem.clone()) {
                stem = format!("{base}-{suffix}");
                suffix += 1;
            }
            (
                stem,
                flat(
                    &a.name,
                    &format!("{:.1}% owned", a.ownership_share * 100.0),
                    "#1769aa",
                ),
            )
        })
        .collect();
    let top = authors.first().filter(|_| total > 0.0).map_or_else(
        || "no data".to_owned(),
        |a| format!("{} {:.0}%", a.name, a.ownership_share * 100.0),
    );
    Ok(Badges {
        bar: bar(&authors, total, options),
        card: card(&authors, report, options),
        donut: charts::donut(&authors, total, options),
        labor: charts::labor(&authors, report, options),
        treemap: treemap::render(&authors, report, options),
        activity: timeline::render(report, options, false),
        history: timeline::render(report, options, true),
        bus_factor: flat("bus factor", &message, color),
        authors: author_badges,
        bus_factor_json: endpoint("bus factor", &message, color),
        top_contributor_json: endpoint("top contributor", &top, "blue"),
    })
}

/// Validate all numbers before sorting, summing, rounding or producing geometry.
fn validate(report: &Report, options: Options) -> Result<(), Error> {
    timeline::validate(report)?;
    if !report.total_lines.is_finite()
        || report.total_lines < 0.0
        || report.directories.iter().any(|d| {
            !d.total_lines.is_finite()
                || d.total_lines < 0.0
                || !d.owner_share.is_finite()
                || !(0.0..=1.0).contains(&d.owner_share)
        })
    {
        return Err(Error::InvalidDirectory);
    }
    if !(1..=1000).contains(&options.top) {
        return Err(Error::InvalidTop);
    }
    if !(200..=4096).contains(&options.width) {
        return Err(Error::InvalidWidth);
    }
    if report.authors.iter().any(|a| {
        !a.ownership_share.is_finite()
            || !(0.0..=1.0).contains(&a.ownership_share)
            || !a.labor.is_finite()
            || a.labor < 0.0
    }) {
        return Err(Error::InvalidAuthor);
    }
    let total: f64 = report.authors.iter().map(|a| a.ownership_share).sum();
    if total != 0.0 && (total - 1.0).abs() > 0.000_001 {
        return Err(Error::InvalidShareTotal);
    }
    Ok(())
}

/// Minimum ranked authors strictly exceeding half of all live ownership.
fn bus_factor(authors: &[&Author], total: f64) -> usize {
    if total == 0.0 {
        return 0;
    }
    let mut sum = 0.0;
    for (i, author) in authors.iter().enumerate() {
        sum += author.ownership_share;
        // Inputs already express shares of the whole. Re-summing their slightly
        // rounded denominator can turn an exact 50% prefix into a false majority.
        if sum > 0.5 {
            return i + 1;
        }
    }
    0
}

/// Color-blind-safe categorical colors in author-rank order, shared with terminals.
/// Light colors are darkened Okabe-Ito hues for contrast on white.
pub const PALETTE: [&str; 8] = [
    "#0072b2", "#a65f00", "#007b5e", "#97558a", "#007f9b", "#b84400", "#827000", "#666666",
];

/// Theme-specific text and categorical colors; labels never rely on color alone.
impl Theme {
    /// Background, foreground and secondary text, respectively.
    fn colors(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Light => ("#fff", "#24292f", "#57606a"),
            Self::Dark => ("#161b22", "#f0f6fc", "#b1bac4"),
        }
    }

    /// Neutral aggregate fill, distinct from rank-based author coloring.
    fn neutral(self) -> &'static str {
        match self {
            Self::Light => "#666666",
            Self::Dark => "#a0a0a0",
        }
    }

    /// Rank-preserving brighter counterparts for dark surfaces.
    pub fn color(self, rank: usize) -> &'static str {
        let palette = match self {
            Self::Light => PALETTE,
            Self::Dark => [
                "#56b4e9", "#e69f00", "#009e73", "#cc79a7", "#66c2d4", "#ed7845", "#d5c45b",
                "#a0a0a0",
            ],
        };
        palette
            .get(rank % palette.len())
            .copied()
            .unwrap_or("#777777")
    }
}

/// Reject bidi formatting and terminal controls while preserving ordinary Unicode.
fn unsafe_scalar(c: char) -> bool {
    c.is_control()
        || matches!(u32::from(c), 0x061c | 0x200e | 0x200f | 0x202a..=0x202e | 0x2066..=0x2069)
}

/// Replace XML 1.0 forbidden scalars and escape all markup/attribute delimiters.
fn xml(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' | '\n' | '\r' => out.push(' '),
            c if unsafe_scalar(c) || c == '\u{fffe}' || c == '\u{ffff}' => out.push('\u{fffd}'),
            c => out.push(c),
        }
    }
    out
}

/// Estimate conservative Verdana 11px advances, with a wide Unicode fallback.
fn advance(c: char) -> u32 {
    match c {
        'i' | 'l' | 'I' | '!' | '.' | ',' | ':' | ';' | '\'' | '|' => 4,
        'm' | 'w' | 'M' | 'W' | '@' | '%' => 11,
        ' ' | '(' | ')' | '[' | ']' | 't' | 'f' | 'r' => 5,
        c if c.is_ascii() => 8,
        _ => 12,
    }
}

/// Measure using saturating arithmetic so arbitrary strings cannot overflow.
fn measure(text: &str) -> u32 {
    text.chars()
        .fold(0_u32, |width, c| width.saturating_add(advance(c)))
}

/// Fit a legend/table label to its column and retain the original in a title.
fn fit(text: &str, width: u32) -> String {
    if measure(text) <= width {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut used = 0_u32;
    for c in text.chars() {
        if used.saturating_add(advance(c)).saturating_add(12) > width {
            break;
        }
        out.push(c);
        used += advance(c);
    }
    out.push_str("...");
    out
}

/// Common SVG 1.1 root and accessible title, without external assets or scripts.
fn svg(width: u32, height: usize, title: &str) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" version=\"1.1\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\" role=\"img\"><title>{}</title>",
        xml(title)
    )
}

/// Append plain escaped text with fixed font metrics and explicit direction.
fn text(out: &mut String, x: impl fmt::Display, y: usize, value: &str, color: &str) {
    let _ = write!(
        out,
        "<text x=\"{x}\" y=\"{y}\" fill=\"{color}\" font-family=\"Verdana,DejaVu Sans,sans-serif\" font-size=\"11\" direction=\"ltr\" unicode-bidi=\"embed\">{}</text>",
        xml(value)
    );
}

/// Shields-style two-segment flat badge with width based on both text segments.
fn flat(label: &str, message: &str, color: &str) -> String {
    let left = measure(label).saturating_add(20);
    let right = measure(message).saturating_add(20);
    let width = left.saturating_add(right);
    let mut out = svg(width, 20, &format!("{label}: {message}"));
    let _ = write!(
        out,
        "<rect width=\"{left}\" height=\"20\" fill=\"#555\"/><rect x=\"{left}\" width=\"{right}\" height=\"20\" fill=\"{color}\"/>"
    );
    text(&mut out, 10, 14, label, "#fff");
    text(&mut out, left.saturating_add(10), 14, message, "#fff");
    out.push_str("</svg>\n");
    out
}

/// Largest-remainder rounding of grouped percentages to exactly 100.0%.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // Validated normalized shares yield bounded 0..=1000 values.
fn percentages(shares: &[f64], total: f64) -> Vec<u32> {
    let exact: Vec<_> = shares.iter().map(|s| s / total * 1000.0).collect();
    let mut units: Vec<_> = exact.iter().map(|n| n.floor() as u32).collect();
    let allocated: u32 = units.iter().sum();
    let mut ranks: Vec<_> = exact.iter().enumerate().collect();
    ranks.sort_by(|a, b| {
        (b.1 - b.1.floor())
            .total_cmp(&(a.1 - a.1.floor()))
            .then(a.0.cmp(&b.0))
    });
    for (i, _) in ranks
        .into_iter()
        .take(1000_u32.saturating_sub(allocated) as usize)
    {
        if let Some(unit) = units.get_mut(i) {
            *unit += 1;
        }
    }
    units
}

/// Stacked top-author ownership bar with a complete rounded percentage legend.
fn bar(authors: &[&Author], total: f64, options: Options) -> String {
    if total == 0.0 {
        return flat("contributions", "no data", "#555");
    }
    let mut groups: Vec<_> = authors
        .iter()
        .take(options.top)
        .map(|a| (a.name.as_str(), a.ownership_share))
        .collect();
    let other: f64 = authors
        .iter()
        .skip(options.top)
        .map(|a| a.ownership_share)
        .sum();
    if other > 0.0 {
        groups.push(("others", other));
    }
    let (bg, fg, _) = options.theme.colors();
    let shares: Vec<_> = groups.iter().map(|(_, share)| *share).collect();
    let units = percentages(&shares, total);
    let mut out = svg(
        options.width,
        42 + groups.len() * 24,
        "Contributions: ownership",
    );
    let _ = write!(
        out,
        "<rect width=\"{}\" height=\"{}\" fill=\"{bg}\"/>",
        options.width,
        42 + groups.len() * 24
    );
    let mut x = 0.0;
    for (i, ((name, share), percent)) in groups.iter().zip(units).enumerate() {
        let color = if i >= options.top {
            options.theme.neutral()
        } else {
            options.theme.color(i)
        };
        let width = share / total * f64::from(options.width);
        let _ = write!(
            out,
            "<rect x=\"{x:.4}\" width=\"{width:.4}\" height=\"20\" fill=\"{color}\"><title>{}</title></rect><rect x=\"12\" y=\"{}\" width=\"12\" height=\"12\" fill=\"{color}\"/>",
            xml(name),
            34 + i * 24
        );
        let label = format!(
            "{} {}.{}%",
            fit(name, options.width.saturating_sub(110)),
            percent / 10,
            percent % 10
        );
        text(&mut out, 32, 44 + i * 24, &label, fg);
        x += width;
    }
    out.push_str("</svg>\n");
    out
}

/// Summary card, keeping ownership independent of historical labor.
fn card(authors: &[&Author], report: &Report, options: Options) -> String {
    let count = authors.len().min(options.top);
    let height = 112 + count * 26;
    let (bg, fg, muted) = options.theme.colors();
    let mut out = svg(options.width, height, "Contributions");
    let _ = write!(
        out,
        "<rect width=\"{}\" height=\"{height}\" rx=\"6\" fill=\"{bg}\"/>",
        options.width
    );
    text(&mut out, 16, 26, "Contributions", fg);
    let owned_x = options.width.saturating_sub(132);
    let labor_x = options.width.saturating_sub(65);
    text(
        &mut out,
        16,
        50,
        &fit("Author", owned_x.saturating_sub(28)),
        muted,
    );
    text(&mut out, owned_x, 50, "Owned", muted);
    text(&mut out, labor_x, 50, "Labor", muted);
    if authors.is_empty() {
        text(&mut out, 16, 76, "no data", muted);
    }
    for (i, author) in authors.iter().take(options.top).enumerate() {
        let y = 76 + i * 26;
        let _ = write!(out, "<g><title>{}</title>", xml(&author.name));
        text(
            &mut out,
            16,
            y,
            &fit(&author.name, owned_x.saturating_sub(28)),
            fg,
        );
        text(
            &mut out,
            owned_x,
            y,
            &format!("{:.1}%", author.ownership_share * 100.0),
            fg,
        );
        text(
            &mut out,
            labor_x,
            y,
            &fit(&format!("{:.1}", author.labor), 53),
            fg,
        );
        let bar_width = author.ownership_share * f64::from(options.width - 32);
        let color = options.theme.color(i);
        let _ = write!(
            out,
            "<rect x=\"16\" y=\"{}\" width=\"{bar_width:.4}\" height=\"4\" fill=\"{color}\"/>",
            y + 5
        );
        out.push_str("</g>");
    }
    let short: String = report.head.chars().take(12).collect();
    let footer = format!("{} commits | HEAD {short}", report.commits);
    text(
        &mut out,
        16,
        height - 16,
        &fit(&footer, options.width - 32),
        muted,
    );
    out.push_str("</svg>\n");
    out
}

/// ASCII-only bounded filename component, including a Windows device-name prefix.
fn slug(name: &str) -> String {
    let mut out = String::from("author-");
    let mut separator = false;
    for c in name.chars() {
        if out.len() >= 72 {
            break;
        }
        if c.is_ascii_alphanumeric() {
            if separator && !out.ends_with('-') {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
            separator = false;
        } else {
            separator = true;
        }
    }
    if out == "author-" {
        out.push_str("anonymous");
    }
    // Reserve the theme suffix to avoid collisions between light and dark author SVGs.
    if out.ends_with("-dark") {
        out.push_str("-author");
    }
    out
}

/// Escape a JSON string with all control characters encoded, without serde.
fn json_string(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c < ' ' => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Version-one shields endpoint, with deterministic field ordering and no timestamp.
fn endpoint(label: &str, message: &str, color: &str) -> String {
    format!(
        "{{\"schemaVersion\":1,\"label\":{},\"message\":{},\"color\":{}}}\n",
        json_string(label),
        json_string(message),
        json_string(color)
    )
}

#[cfg(test)]
mod tests;
