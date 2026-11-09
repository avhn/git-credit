//! Pure historical charts and weekly terminal activity, with Gregorian date axes.

use std::{collections::BTreeMap, fmt::Write};

use super::{Error, Options, Report, charts, fit, measure, text, xml};

/// Optional historical series with privacy-filtered report-local author references.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Timeline {
    /// Sparse UTC Monday activity buckets, ordered by date.
    pub activity: Vec<Activity>,
    /// Ownership samples ordered along the first-parent ancestry.
    pub ownership: Vec<Ownership>,
}

/// Exact fixed-point amount for one author in the input report's original order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Amount {
    /// Index into `Report::authors`, not a display rank or a cache identity.
    pub author: usize,
    /// Millionths of a meaningful line.
    pub amount: u64,
}

/// Weekly meaningful labor; empty weeks may be omitted in the input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Activity {
    /// UTC Monday date, beginning with YYYY-MM-DD.
    pub week_start: String,
    /// Total labor in millionths of a line, equal to the author sum.
    pub total: u64,
    /// Sparse, uniquely indexed author amounts.
    pub authors: Vec<Amount>,
}

/// Absolute ownership at one sample on the main line of history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ownership {
    /// Increasing first-parent position, independent of author timestamps.
    pub first_parent_index: usize,
    /// Author timestamp beginning with YYYY-MM-DD; dates may go backwards.
    pub author_timestamp: String,
    /// Total ownership in millionths of a line, equal to the author sum.
    pub total: u64,
    /// Sparse, uniquely indexed author amounts.
    pub authors: Vec<Amount>,
}

/// Gregorian date with a bounded year, allowing deterministic gap filling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Date {
    year: i32,
    month: i32,
    day: i32,
}

impl Date {
    /// Parse an ASCII ISO date prefix, rejecting impossible calendar dates.
    fn parse(value: &str) -> Result<Self, Error> {
        let bytes = value.as_bytes();
        if bytes.len() < 10 || bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
            return Err(Error::InvalidTimeline);
        }
        let number = |range: std::ops::Range<usize>| -> Result<i32, Error> {
            bytes
                .get(range)
                .ok_or(Error::InvalidTimeline)?
                .iter()
                .try_fold(0, |n, b| {
                    if b.is_ascii_digit() {
                        Ok(n * 10 + i32::from(b - b'0'))
                    } else {
                        Err(Error::InvalidTimeline)
                    }
                })
        };
        let date = Self {
            year: number(0..4)?,
            month: number(5..7)?,
            day: number(8..10)?,
        };
        if !(1..=12).contains(&date.month) || !(1..=date.month_days()).contains(&date.day) {
            return Err(Error::InvalidTimeline);
        }
        Ok(date)
    }

    /// Calendar month length, including Gregorian century leap-year rules.
    fn month_days(self) -> i32 {
        match self.month {
            2 if self.year % 4 == 0 && (self.year % 100 != 0 || self.year % 400 == 0) => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        }
    }

    /// Days before a year, taking year zero as a leap year.
    fn year_days(year: i32) -> i32 {
        365 * year + (year + 3) / 4 - (year + 99) / 100 + (year + 399) / 400
    }

    /// Ordinal day since year zero; all supported dates fit exactly in i32.
    fn ordinal(self) -> i32 {
        let months: i32 = (1..self.month)
            .map(|month| Self { month, ..self }.month_days())
            .sum();
        Self::year_days(self.year) + months + self.day - 1
    }

    /// Invert an ordinal using a bounded year search and at most twelve months.
    fn from_ordinal(ordinal: i32) -> Self {
        let mut low = 0;
        let mut high = 10_000;
        while high - low > 1 {
            let mid = i32::midpoint(low, high);
            if Self::year_days(mid) <= ordinal {
                low = mid;
            } else {
                high = mid;
            }
        }
        let mut date = Self {
            year: low,
            month: 1,
            day: 1,
        };
        let mut remaining = ordinal - Self::year_days(low);
        while remaining >= date.month_days() && date.month < 12 {
            remaining -= date.month_days();
            date.month += 1;
        }
        date.day = remaining + 1;
        date
    }

    /// Fixed-width ASCII calendar label for a weekly column.
    fn label(self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// Consecutive calendar month key, anchored at year zero.
    fn month_key(self) -> i32 {
        self.year * 12 + self.month - 1
    }
}

/// Fail closed on invalid dates, duplicate authors, overflow or broken conservation.
pub(super) fn validate(report: &Report) -> Result<(), Error> {
    let Some(timeline) = &report.timeline else {
        return Ok(());
    };
    let amounts = |values: &[Amount], total: u64| {
        let mut seen = std::collections::BTreeSet::new();
        let sum = values.iter().try_fold(0_u64, |sum, value| {
            if value.author >= report.authors.len() || !seen.insert(value.author) {
                return None;
            }
            sum.checked_add(value.amount)
        });
        if sum == Some(total) {
            Ok(())
        } else {
            Err(Error::InvalidTimeline)
        }
    };
    let mut previous = None;
    for week in &timeline.activity {
        let day = Date::parse(&week.week_start)?.ordinal();
        // 0000-01-03 was Monday; sparse weeks must still share this alignment.
        if (day - 2).rem_euclid(7) != 0 || previous.is_some_and(|p| p >= day) {
            return Err(Error::InvalidTimeline);
        }
        previous = Some(day);
        amounts(&week.authors, week.total)?;
    }
    let mut previous = None;
    for sample in &timeline.ownership {
        Date::parse(&sample.author_timestamp)?;
        if previous.is_some_and(|p| p >= sample.first_parent_index) {
            return Err(Error::InvalidTimeline);
        }
        previous = Some(sample.first_parent_index);
        amounts(&sample.authors, sample.total)?;
    }
    Ok(())
}

/// Display-only conversion; exact integers are retained until geometry is needed.
#[allow(clippy::cast_precision_loss)] // SVG geometry is approximate, never attribution accounting.
fn lines(amount: u64) -> f64 {
    amount as f64 / 1_000_000.0
}

/// One plotted point, including explicit empty calendar buckets.
struct Point {
    label: String,
    values: Vec<f64>,
}

/// Preserve the shared ownership rank even for labor-only historical contributors.
fn ranks(report: &Report) -> Vec<usize> {
    let mut indices: Vec<_> = (0..report.authors.len()).collect();
    indices.sort_by(|a, b| {
        let (a, b) = (&report.authors[*a], &report.authors[*b]);
        b.ownership_share
            .total_cmp(&a.ownership_share)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| b.labor.total_cmp(&a.labor))
    });
    indices
}

/// Group amounts in stable author order; all unselected identities use one aggregate.
fn grouped(amounts: &[Amount], mapping: &[usize], count: usize) -> Vec<f64> {
    let mut values = vec![0.0; count + 1];
    let mut sorted: Vec<_> = amounts.iter().collect();
    sorted.sort_by_key(|v| v.author);
    for amount in sorted {
        if let Some(value) = mapping
            .get(amount.author)
            .and_then(|rank| values.get_mut((*rank).min(count)))
        {
            *value += lines(amount.amount);
        }
    }
    values
}

/// Use dense weekly spacing, switching to calendar months above 150 weeks.
fn activity_points(timeline: &Timeline, mapping: &[usize], count: usize) -> (Vec<Point>, bool) {
    let Some(first) = timeline
        .activity
        .first()
        .and_then(|v| Date::parse(&v.week_start).ok())
    else {
        return (Vec::new(), false);
    };
    let Some(last) = timeline
        .activity
        .last()
        .and_then(|v| Date::parse(&v.week_start).ok())
    else {
        return (Vec::new(), false);
    };
    let monthly = (last.ordinal() - first.ordinal()) / 7 + 1 > 150;
    let key = |date: Date| {
        if monthly {
            date.month_key()
        } else {
            date.ordinal() / 7
        }
    };
    let mut buckets: BTreeMap<i32, Vec<f64>> = BTreeMap::new();
    for week in &timeline.activity {
        if let Ok(date) = Date::parse(&week.week_start) {
            let bucket = buckets
                .entry(key(date))
                .or_insert_with(|| vec![0.0; count + 1]);
            for (target, amount) in bucket
                .iter_mut()
                .zip(grouped(&week.authors, mapping, count))
            {
                *target += amount;
            }
        }
    }
    let points = (key(first)..=key(last))
        .map(|key| {
            let label = if monthly {
                format!("{:04}-{:02}", key / 12, key % 12 + 1)
            } else {
                Date::from_ordinal(key * 7 + 2).label()
            };
            Point {
                label,
                values: buckets.remove(&key).unwrap_or_else(|| vec![0.0; count + 1]),
            }
        })
        .collect();
    (points, monthly)
}

/// Nice nonnegative y ticks with about four intervals, also for fractional lines.
fn y_ticks(maximum: f64) -> Vec<f64> {
    if maximum <= 0.0 {
        return vec![0.0, 1.0];
    }
    let rough = maximum / 4.0;
    let power = 10.0_f64.powf(rough.log10().floor());
    let fraction = rough / power;
    let step = [1.0, 2.0, 2.5, 5.0, 10.0]
        .into_iter()
        .find(|n| *n >= fraction)
        .unwrap_or(10.0)
        * power;
    (0..=4).map(|n| f64::from(n) * step).collect()
}

/// Compact tick values preserve small fractions and large repository magnitudes.
fn tick_label(value: f64) -> String {
    if value == 0.0 {
        "0".into()
    } else if !(0.01..1_000_000.0).contains(&value) {
        format!("{value:.1e}")
    } else {
        format!("{value:.2}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    }
}

/// Date ticks select month or year labels by span and enforce measured separation.
fn x_ticks(points: &[Point], width: u32, history: bool) -> Vec<(usize, String)> {
    let date_years: Vec<_> = points
        .iter()
        .filter_map(|point| point.label.get(..4)?.parse::<i32>().ok())
        .collect();
    let years = date_years
        .iter()
        .min()
        .zip(date_years.iter().max())
        .is_some_and(|(first, last)| last - first >= 2);
    let mut ticks = Vec::new();
    let mut previous = String::new();
    let mut right = -1000.0_f64;
    for (index, point) in points.iter().enumerate() {
        let len = if years { 4 } else { 7 };
        let label = point.label.get(..len).unwrap_or(&point.label).to_owned();
        if label == previous {
            continue;
        }
        previous.clone_from(&label);
        let position = position(index, points.len(), f64::from(width), history);
        let half = f64::from(measure(&label)) / 2.0;
        if position - half >= right + 12.0 {
            right = position + half;
            ticks.push((index, label));
        }
    }
    ticks
}

/// Columns use bucket centers; ownership uses evenly spaced sample sequence points.
fn position(index: usize, count: usize, width: f64, history: bool) -> f64 {
    let index = f64::from(u32::try_from(index).unwrap_or(u32::MAX));
    let count = f64::from(u32::try_from(count).unwrap_or(u32::MAX));
    if history && count > 1.0 {
        index / (count - 1.0) * width
    } else {
        (index + 0.5) / count.max(1.0) * width
    }
}

/// Bounded display coordinates for the historical plot.
struct Plot {
    left: f64,
    bottom: f64,
    width: f64,
    ceiling: f64,
}

/// Draw stacked areas or columns, preserving tiny amounts without inflation.
fn stacks(
    out: &mut String,
    points: &[Point],
    active: &[usize],
    plot: &Plot,
    history: bool,
    color: impl Fn(usize) -> &'static str,
) {
    if history && points.len() > 1 {
        let mut baseline = vec![0.0; points.len()];
        for rank in active {
            let mut path = String::new();
            for (i, (point, base)) in points.iter().zip(&baseline).enumerate() {
                let x = plot.left + position(i, points.len(), plot.width, true);
                let y = plot.bottom
                    - (base + point.values.get(*rank).copied().unwrap_or(0.0)) / plot.ceiling
                        * 190.0;
                let _ = write!(path, "{} {x:.3} {y:.3} ", if i == 0 { "M" } else { "L" });
            }
            for (i, base) in baseline.iter().enumerate().rev() {
                let x = plot.left + position(i, points.len(), plot.width, true);
                let y = plot.bottom - base / plot.ceiling * 190.0;
                let _ = write!(path, "L {x:.3} {y:.3} ");
            }
            let _ = write!(out, "<path d=\"{path}Z\" fill=\"{}\"/>", color(*rank));
            for (base, point) in baseline.iter_mut().zip(points) {
                *base += point.values.get(*rank).copied().unwrap_or(0.0);
            }
        }
    } else {
        let cells = f64::from(u32::try_from(points.len()).unwrap_or(u32::MAX));
        let bar_width = (plot.width / cells * 0.82).min(48.0);
        for (i, point) in points.iter().enumerate() {
            let x = plot.left + position(i, points.len(), plot.width, false) - bar_width / 2.0;
            let mut base = 0.0;
            for rank in active {
                let amount = point.values.get(*rank).copied().unwrap_or(0.0);
                if amount == 0.0 {
                    continue;
                }
                let h = amount / plot.ceiling * 190.0;
                let y = plot.bottom - base / plot.ceiling * 190.0 - h;
                let _ = write!(
                    out,
                    "<rect x=\"{x:.3}\" y=\"{y:.3}\" width=\"{bar_width:.3}\" height=\"{h:.6}\" fill=\"{}\"><title>{}: {amount:.2} lines</title></rect>",
                    color(*rank),
                    xml(&point.label)
                );
                base += amount;
            }
        }
    }
}

/// Preserve sample order and values, using the latest UTC timestamp reached for labels.
/// ISO date prefixes sort chronologically, even when raw sample dates go backwards.
fn ownership_points(timeline: &Timeline, mapping: &[usize], count: usize) -> Vec<Point> {
    timeline
        .ownership
        .iter()
        .scan("", |latest, sample| {
            *latest = (*latest).max(sample.author_timestamp.as_str());
            Some(Point {
                label: (*latest).to_owned(),
                values: grouped(&sample.authors, mapping, count),
            })
        })
        .collect()
}

/// Resolve rank-indexed sparse history into dense display points.
fn prepare(
    report: &Report,
    options: Options,
    history: bool,
) -> (Vec<usize>, usize, Vec<Point>, bool) {
    let indices = ranks(report);
    let count = options.top.min(indices.len());
    let mut mapping = vec![count; indices.len()];
    for (rank, index) in indices.iter().enumerate() {
        if let Some(value) = mapping.get_mut(*index) {
            *value = rank;
        }
    }
    let (points, monthly) = report.timeline.as_ref().map_or_else(
        || (Vec::new(), false),
        |timeline| {
            if history {
                (ownership_points(timeline, &mapping, count), false)
            } else {
                activity_points(timeline, &mapping, count)
            }
        },
    );
    (indices, count, points, monthly)
}

/// Fit visible names while retaining complete, escaped legend titles.
fn legend(
    out: &mut String,
    report: &Report,
    indices: &[usize],
    active: &[usize],
    options: Options,
    legend_top: usize,
) {
    let count = options.top.min(indices.len());
    let (_, fg, _) = options.theme.colors();
    let color = |rank| {
        if rank == count {
            options.theme.neutral()
        } else {
            options.theme.color(rank)
        }
    };
    for (row, rank) in active.iter().enumerate() {
        let name = indices
            .get(*rank)
            .and_then(|i| report.authors.get(*i))
            .filter(|_| *rank < count)
            .map_or("Others", |a| a.name.as_str());
        let y = legend_top + row * 22;
        let _ = write!(
            out,
            "<g><title>{}</title><rect x=\"16\" y=\"{}\" width=\"12\" height=\"12\" fill=\"{}\"/>",
            xml(name),
            y - 10,
            color(*rank)
        );
        text(out, 36, y, &fit(name, options.width - 52), fg);
        out.push_str("</g>");
    }
}

/// Render aligned numeric tick labels and quiet horizontal grid lines.
fn y_axis(
    out: &mut String,
    ticks: &[f64],
    left: u32,
    bottom: usize,
    width: f64,
    ceiling: f64,
    muted: &str,
) {
    for &value in ticks {
        let y = f64::from(u32::try_from(bottom).unwrap_or(u32::MAX)) - value / ceiling * 190.0;
        let _ = write!(
            out,
            "<path d=\"M {left} {y:.3} h {width:.3}\" stroke=\"{muted}\" stroke-opacity=\"0.2\"/>"
        );
        let label = tick_label(value);
        let _ = write!(
            out,
            "<text x=\"{}\" y=\"{:.3}\" text-anchor=\"end\" fill=\"{muted}\" font-family=\"Verdana,DejaVu Sans,sans-serif\" font-size=\"11\">{}</text>",
            left - 8,
            y + 4.0,
            xml(&label)
        );
    }
}

/// Draw absolute historical stacks with shared author colors and bounded legends.
pub(super) fn render(report: &Report, options: Options, history: bool) -> String {
    let (indices, count, points, monthly) = prepare(report, options, history);
    let title = if history {
        "Ownership over time"
    } else {
        "Meaningful activity"
    };
    let subtitle = if history {
        "Absolute owned lines; dates are the latest commit time reached on the main line"
    } else if monthly {
        "Monthly labor (aggregated by UTC week start); empty months are gaps"
    } else {
        "Weekly labor; UTC Monday buckets, empty weeks are gaps"
    };
    let subtitles = charts::wrap(subtitle, options.width - 32);
    let top = 56 + subtitles.len() * 18;
    let bottom = top + 190;
    let legend_top = bottom + 58;
    let active: Vec<_> = (0..=count)
        .filter(|rank| {
            points
                .iter()
                .any(|p| p.values.get(*rank).is_some_and(|v| *v > 0.0))
        })
        .collect();
    let mut out = charts::canvas(options, legend_top + active.len() * 22 + 12, title);
    let (_, fg, muted) = options.theme.colors();
    text(&mut out, 16, 25, title, fg);
    for (i, line) in subtitles.iter().enumerate() {
        text(&mut out, 16, 45 + i * 18, line, muted);
    }
    if points.is_empty() {
        text(&mut out, 16, top + 35, "no data", fg);
        out.push_str("</svg>\n");
        return out;
    }
    let maximum = points
        .iter()
        .map(|p| p.values.iter().sum::<f64>())
        .fold(0.0_f64, f64::max);
    let ticks = y_ticks(maximum);
    let ceiling = ticks.last().copied().unwrap_or(1.0);
    let left = ticks
        .iter()
        .map(|v| measure(&tick_label(*v)))
        .max()
        .unwrap_or(0)
        + 24;
    let plot_width = options.width.saturating_sub(left + 36);
    let width = f64::from(plot_width);
    text(
        &mut out,
        16,
        top - 12,
        if history {
            "owned lines"
        } else {
            "labor (lines)"
        },
        fg,
    );
    y_axis(&mut out, &ticks, left, bottom, width, ceiling, muted);
    let color = |rank| {
        if rank == count {
            options.theme.neutral()
        } else {
            options.theme.color(rank)
        }
    };
    let plot = Plot {
        left: f64::from(left),
        bottom: f64::from(u32::try_from(bottom).unwrap_or(u32::MAX)),
        width,
        ceiling,
    };
    stacks(&mut out, &points, &active, &plot, history, color);
    for (index, label) in x_ticks(&points, plot_width, history) {
        let x = f64::from(left) + position(index, points.len(), width, history);
        let _ = write!(
            out,
            "<text x=\"{x:.3}\" y=\"{}\" text-anchor=\"middle\" fill=\"{muted}\" font-family=\"Verdana,DejaVu Sans,sans-serif\" font-size=\"11\">{}</text>",
            bottom + 22,
            xml(&label)
        );
    }
    legend(&mut out, report, &indices, &active, options, legend_top);
    out.push_str("</svg>\n");
    out
}

/// Render the last 52 calendar weeks as one labeled, peak-preserving activity line.
///
/// Sparse input is filled with zero weeks. Narrow output coalesces adjacent weeks
/// using their maximum, not their sum; the peak label always describes one week.
/// `columns` is the desired total character width, at least 40. Below 70 columns
/// dates use year-month labels; below 50 the redundant line-unit suffix is omitted.
///
/// # Errors
/// Returns `InvalidTimeline` for malformed or non-increasing week dates.
pub fn activity_sparkline(
    weeks: &[(&str, u64)],
    ascii: bool,
    columns: usize,
) -> Result<String, Error> {
    let mut parsed = BTreeMap::new();
    let mut previous = None;
    for (label, total) in weeks {
        let day = Date::parse(label)?.ordinal();
        if (day - 2).rem_euclid(7) != 0 || previous.is_some_and(|p| p >= day) {
            return Err(Error::InvalidTimeline);
        }
        previous = Some(day);
        parsed.insert(day, *total);
    }
    let Some((&last, _)) = parsed.last_key_value() else {
        return Ok("Activity: no data".into());
    };
    let first = parsed
        .first_key_value()
        .map_or(last, |(day, _)| *day)
        .max(last - 51 * 7);
    let values: Vec<_> = (first..=last)
        .step_by(7)
        .map(|day| parsed.get(&day).copied().unwrap_or(0))
        .collect();
    let peak = values.iter().copied().max().unwrap_or(0);
    let date_label = |day| {
        let label = Date::from_ordinal(day).label();
        if columns < 70 {
            label.get(..7).unwrap_or(&label).to_owned()
        } else {
            label
        }
    };
    let units = if columns < 50 { "" } else { " lines" };
    let label = format!(
        "Activity {}..{} peak {}{units} ",
        date_label(first),
        date_label(last),
        tick_label(lines(peak))
    );
    let cells = columns
        .saturating_sub(label.chars().count())
        .min(values.len())
        .max(1);
    let levels = if ascii {
        [' ', '.', ':', '-', '=', '+', '*', '#', '@']
    } else {
        [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█']
    };
    let mut out = label;
    for cell in 0..cells {
        let start = cell * values.len() / cells;
        let end = ((cell + 1) * values.len() / cells).max(start + 1);
        let value = values
            .get(start..end)
            .and_then(|v| v.iter().max())
            .copied()
            .unwrap_or(0);
        let level = if peak == 0 || value == 0 {
            0
        } else {
            usize::try_from((u128::from(value) * 8).div_ceil(u128::from(peak))).unwrap_or(8)
        };
        out.push(levels.get(level).copied().unwrap_or(' '));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Calendar conversion includes leap centuries and the supported endpoints.
    #[test]
    fn calendar_round_trips_and_rejects_invalid_dates() {
        for label in [
            "0000-01-01",
            "1900-03-01",
            "2000-02-29",
            "2020-12-31",
            "9999-12-31",
        ] {
            let date = Date::parse(label).unwrap();
            assert_eq!(Date::from_ordinal(date.ordinal()).label(), label);
        }
        for label in [
            "2021-02-29",
            "1900-02-29",
            "2020-00-01",
            "2020-12-32",
            "שלוםשלום",
        ] {
            assert!(Date::parse(label).is_err());
        }
    }

    /// Nice ticks cover the maximum without losing fractional meaningful lines.
    #[test]
    fn nice_y_ticks_cover_small_and_large_values() {
        for maximum in [0.0, 0.000_001, 0.3, 1.0, 7.0, 999.0, 1e12] {
            let ticks = y_ticks(maximum);
            assert_eq!(ticks[0], 0.0);
            assert!(*ticks.last().unwrap() >= maximum);
            assert!(ticks.windows(2).all(|pair| pair[1] > pair[0]));
        }
        assert_eq!(y_ticks(7.0), [0.0, 2.0, 4.0, 6.0, 8.0]);
    }

    /// Sparse UTC weeks fill gaps and month assignment uses the Monday date.
    #[test]
    fn aggregation_switches_at_151_weeks_and_keeps_month_boundaries() {
        let first = Date::parse("2020-01-27").unwrap().ordinal();
        let mut timeline = Timeline::default();
        for offset in [0, 1, 149] {
            timeline.activity.push(Activity {
                week_start: Date::from_ordinal(first + offset * 7).label(),
                total: 1_000_000,
                authors: vec![Amount {
                    author: 0,
                    amount: 1_000_000,
                }],
            });
        }
        let (weekly, monthly) = activity_points(&timeline, &[0], 1);
        assert!(!monthly);
        assert_eq!(weekly.len(), 150);
        assert_eq!(weekly[2].values, [0.0, 0.0]);
        timeline.activity.last_mut().unwrap().week_start =
            Date::from_ordinal(first + 150 * 7).label();
        let (months, monthly) = activity_points(&timeline, &[0], 1);
        assert!(monthly);
        assert_eq!(months[0].label, "2020-01");
        assert_eq!(months[0].values[0], 1.0);
        assert_eq!(months[1].label, "2020-02");
        assert_eq!(months[1].values[0], 1.0);
        assert_eq!(months[2].values[0], 0.0);
    }

    /// Measured x tick separation works for narrow canvases and long spans.
    #[test]
    fn date_ticks_never_overlap() {
        let points: Vec<_> = (0..120)
            .map(|i| Point {
                label: format!("{:04}-{:02}-01", 2000 + i / 12, i % 12 + 1),
                values: vec![],
            })
            .collect();
        for width in [90, 200, 500, 4000] {
            for history in [false, true] {
                let ticks = x_ticks(&points, width, history);
                for pair in ticks.windows(2) {
                    let distance = position(pair[1].0, points.len(), f64::from(width), history)
                        - position(pair[0].0, points.len(), f64::from(width), history);
                    assert!(
                        distance
                            >= f64::from(measure(&pair[0].1) + measure(&pair[1].1)) / 2.0 + 12.0
                    );
                }
            }
        }
    }

    /// Backdated samples cannot repeat periods; ticks mark each period's first entry.
    #[test]
    fn ownership_ticks_increase_strictly_despite_backdated_samples() {
        for (dates, expected) in [
            (
                vec![
                    "2017-01-01",
                    "2018-02-01",
                    "2019-03-01",
                    "2018-05-01",
                    "2020-06-01",
                    "2020-01-01",
                    "2023-01-01",
                    "2025-01-01",
                ],
                vec![
                    (0, "2017"),
                    (1, "2018"),
                    (2, "2019"),
                    (4, "2020"),
                    (6, "2023"),
                    (7, "2025"),
                ],
            ),
            (
                vec![
                    "2020-01-06",
                    "2020-03-02",
                    "2020-02-03",
                    "2020-03-09",
                    "2020-05-04",
                ],
                vec![(0, "2020-01"), (1, "2020-03"), (4, "2020-05")],
            ),
        ] {
            let timeline = Timeline {
                activity: Vec::new(),
                ownership: dates
                    .iter()
                    .enumerate()
                    .map(|(index, date)| {
                        let amount = u64::try_from(index).unwrap() + 1;
                        Ownership {
                            first_parent_index: index * 8,
                            author_timestamp: format!("{date}T12:00:00+00:00"),
                            total: amount,
                            authors: vec![Amount { author: 0, amount }],
                        }
                    })
                    .collect(),
            };
            let original = timeline.clone();
            let points = ownership_points(&timeline, &[0], 1);
            assert_eq!(timeline, original);
            assert_eq!(points.len(), dates.len());
            for (point, sample) in points.iter().zip(&timeline.ownership) {
                assert_eq!(point.values, [lines(sample.total), 0.0]);
            }
            assert!(points.windows(2).all(|pair| pair[0].label <= pair[1].label));
            let wide = x_ticks(&points, 4096, true);
            let expected: Vec<_> = expected
                .into_iter()
                .map(|(index, label)| (index, label.to_owned()))
                .collect();
            assert_eq!(wide, expected);
            for width in [90, 200, 500, 4096] {
                let ticks = x_ticks(&points, width, true);
                assert!(ticks.windows(2).all(|pair| pair[0].1 < pair[1].1));
            }
        }
    }

    /// Width, gaps, peak scaling and ASCII fallback are independent of locale.
    #[test]
    fn sparkline_is_bounded_and_preserves_calendar_gaps() {
        let weeks = [("2020-01-06", 1_000_000), ("2021-01-04", 8_000_000)];
        for columns in [40, 60, 80, 120] {
            let unicode = activity_sparkline(&weeks, false, columns).unwrap();
            let ascii = activity_sparkline(&weeks, true, columns).unwrap();
            assert!(unicode.chars().count() <= columns);
            assert!(ascii.len() <= columns);
            assert!(ascii.is_ascii());
            assert!(unicode.ends_with('█'));
            assert!(ascii.ends_with('@'));
            assert!(unicode.contains("peak 8"));
            let large = activity_sparkline(&[("2020-01-06", u64::MAX)], true, columns).unwrap();
            assert!(large.len() <= columns);
        }
        assert_eq!(
            activity_sparkline(&[], true, 80).unwrap(),
            "Activity: no data"
        );
        assert!(activity_sparkline(&[("bad", 1)], true, 80).is_err());
    }
}
