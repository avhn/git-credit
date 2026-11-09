//! Proportional ownership and independently scaled historical-work charts.

use super::{Author, Options, Report, fit, measure, percentages, svg, text, xml};
use std::fmt::Write;

/// Start an accessible themed chart with an opaque background.
pub(super) fn canvas(options: Options, height: usize, title: &str) -> String {
    let mut out = svg(options.width, height, title);
    let (bg, _, _) = options.theme.colors();
    let _ = write!(
        out,
        "<rect width=\"{}\" height=\"{height}\" fill=\"{bg}\"/>",
        options.width
    );
    out
}

/// Group ranked authors without changing their geometric proportions.
fn groups<'a>(authors: &[&'a Author], top: usize) -> Vec<(&'a str, f64)> {
    let mut result: Vec<_> = authors
        .iter()
        .take(top)
        .filter(|a| a.ownership_share > 0.0)
        .map(|a| (a.name.as_str(), a.ownership_share))
        .collect();
    let others: f64 = authors.iter().skip(top).map(|a| a.ownership_share).sum();
    if others > 0.0 {
        result.push(("Others", others));
    }
    result
}

/// Render exact angular shares, including a nondegenerate circle for one owner.
pub(super) fn donut(authors: &[&Author], total: f64, options: Options) -> String {
    let groups = groups(authors, options.top);
    let mut out = canvas(options, 210 + groups.len() * 24, "Ownership donut");
    let (_, fg, muted) = options.theme.colors();
    if total == 0.0 {
        text(&mut out, 16, 30, "no data", fg);
    } else {
        let cx = f64::from(options.width) / 2.0;
        let radius = 66.0;
        let mut angle = -std::f64::consts::FRAC_PI_2;
        let shares: Vec<_> = groups.iter().map(|(_, share)| *share).collect();
        for (i, ((name, share), percent)) in
            groups.iter().zip(percentages(&shares, total)).enumerate()
        {
            let color = if i >= options.top {
                options.theme.neutral()
            } else {
                options.theme.color(i)
            };
            let end = angle + share / total * std::f64::consts::TAU;
            let label = format!("{name}: {}.{}%", percent / 10, percent % 10);
            if groups.len() == 1 {
                let _ = write!(
                    out,
                    "<circle cx=\"{cx}\" cy=\"96\" r=\"{radius}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"28\"><title>{}</title></circle>",
                    xml(&label)
                );
            } else {
                let (x1, y1) = (cx + radius * angle.cos(), 96.0 + radius * angle.sin());
                let (x2, y2) = (cx + radius * end.cos(), 96.0 + radius * end.sin());
                let large = u8::from(end - angle > std::f64::consts::PI);
                let _ = write!(
                    out,
                    "<path d=\"M {x1} {y1} A {radius} {radius} 0 {large} 1 {x2} {y2}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"28\"><title>{}</title></path>",
                    xml(&label)
                );
            }
            let _ = write!(
                out,
                "<rect x=\"16\" y=\"{}\" width=\"12\" height=\"12\" fill=\"{color}\"/>",
                196 + i * 24
            );
            let legend = format!(
                "{}: {}.{}%",
                fit(name, options.width.saturating_sub(120)),
                percent / 10,
                percent % 10
            );
            text(&mut out, 36, 206 + i * 24, &legend, fg);
            angle = end;
        }
        let count = authors.iter().filter(|a| a.ownership_share > 0.0).count();
        let label = count.to_string();
        text(
            &mut out,
            cx - f64::from(measure(&label)) / 2.0,
            92,
            &label,
            fg,
        );
        let noun = if count == 1 { "owner" } else { "owners" };
        text(
            &mut out,
            cx - f64::from(measure(noun)) / 2.0,
            110,
            noun,
            muted,
        );
    }
    out.push_str("</svg>\n");
    out
}

/// Wrap explanatory words without clipping meaning on narrow charts.
pub(super) fn wrap(value: &str, width: u32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in value.split_whitespace() {
        let candidate = if line.is_empty() {
            word.to_owned()
        } else {
            format!("{line} {word}")
        };
        if measure(&candidate) > width && !line.is_empty() {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Preserve a complete numeric value, switching to scientific notation if necessary.
fn value_label(value: f64) -> String {
    let decimal = format!("{value:.1}");
    if measure(&decimal) <= 88 {
        decimal
    } else {
        format!("{value:.2e}")
    }
}

/// Two explicitly labeled bars in the same line units and on the same scale.
pub(super) fn labor(authors: &[&Author], report: &Report, options: Options) -> String {
    let selected: Vec<_> = authors.iter().take(options.top).collect();
    let legend: Vec<_> = [
        "Ownership: code that survives now",
        "Labor: all meaningful work, including later rewritten",
        "Values in lines",
    ]
    .into_iter()
    .flat_map(|label| wrap(label, options.width - 32))
    .collect();
    let first_row = 42 + legend.len() * 18;
    let mut out = canvas(
        options,
        first_row + 20 + selected.len() * 78,
        "Ownership and labor (lines)",
    );
    let (_, fg, _) = options.theme.colors();
    for (i, label) in legend.iter().enumerate() {
        text(&mut out, 16, 24 + i * 18, label, fg);
    }
    if selected.is_empty() {
        text(&mut out, 16, first_row, "no data", fg);
    }
    let maximum = selected
        .iter()
        .flat_map(|a| [a.ownership_share * report.total_lines, a.labor])
        .fold(0.0_f64, f64::max);
    let value_width = selected
        .iter()
        .flat_map(|a| [a.ownership_share * report.total_lines, a.labor])
        .map(|v| measure(&value_label(v)))
        .max()
        .unwrap_or(0);
    let available = f64::from(options.width.saturating_sub(80 + value_width));
    for (i, author) in selected.iter().enumerate() {
        let y = first_row + i * 78;
        let color = options.theme.color(i);
        let _ = write!(out, "<g><title>{}</title>", xml(&author.name));
        text(&mut out, 16, y, &fit(&author.name, options.width - 32), fg);
        for (offset, label, value, outline) in [
            (
                0,
                "Owned",
                author.ownership_share * report.total_lines,
                false,
            ),
            (25, "Labor", author.labor, true),
        ] {
            let width = if maximum > 0.0 {
                value / maximum * available
            } else {
                0.0
            };
            let fill = if outline { "none" } else { color };
            let _ = write!(
                out,
                "<rect x=\"60\" y=\"{}\" width=\"{width:.6}\" height=\"12\" fill=\"{fill}\" stroke=\"{color}\" stroke-width=\"1\"><title>{label}: {value:.1} lines</title></rect>",
                y + 7 + offset
            );
            text(&mut out, 16, y + 17 + offset, label, fg);
            text(
                &mut out,
                64.0 + width,
                y + 17 + offset,
                &value_label(value),
                fg,
            );
        }
        out.push_str("</g>");
    }
    out.push_str(
        "</svg>
",
    );
    out
}
