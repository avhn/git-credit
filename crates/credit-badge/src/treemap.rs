//! Deterministic squarification of a flat directory snapshot.

use super::{Author, Options, Report, charts::canvas, measure, text, xml};
use std::fmt::Write;

/// Remaining canvas or one laid-out tile, in SVG pixels.
#[derive(Clone, Copy, Debug)]
struct Rect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

/// Worst aspect ratio of a proposed row on the remaining canvas's short side.
fn worst(areas: &[f64], side: f64) -> f64 {
    let sum: f64 = areas.iter().sum();
    let min = areas.iter().copied().fold(f64::INFINITY, f64::min);
    let max = areas.iter().copied().fold(0.0_f64, f64::max);
    (side * side * max / (sum * sum)).max(sum * sum / (side * side * min))
}

/// Consume one row along the short side without introducing spacing distortions.
fn row(areas: &[f64], remaining: &mut Rect, tiles: &mut Vec<Rect>) {
    let sum: f64 = areas.iter().sum();
    let vertical = remaining.width >= remaining.height;
    let side = if vertical {
        remaining.height
    } else {
        remaining.width
    };
    let thickness = sum / side;
    let mut cursor = if vertical { remaining.y } else { remaining.x };
    for area in areas {
        let length = area / thickness;
        tiles.push(if vertical {
            Rect {
                x: remaining.x,
                y: cursor,
                width: thickness,
                height: length,
            }
        } else {
            Rect {
                x: cursor,
                y: remaining.y,
                width: length,
                height: thickness,
            }
        });
        cursor += length;
    }
    if vertical {
        remaining.x += thickness;
        remaining.width = (remaining.width - thickness).max(0.0);
    } else {
        remaining.y += thickness;
        remaining.height = (remaining.height - thickness).max(0.0);
    }
}

/// Greedily extend rows only while their worst aspect ratio improves.
fn squarify(areas: &[f64], mut remaining: Rect) -> Vec<Rect> {
    let mut tiles = Vec::with_capacity(areas.len());
    let mut start = 0;
    while start < areas.len() {
        let mut end = start + 1;
        let side = remaining.width.min(remaining.height);
        while end < areas.len()
            && worst(&areas[start..=end], side) <= worst(&areas[start..end], side)
        {
            end += 1;
        }
        row(&areas[start..end], &mut remaining, &mut tiles);
        start = end;
    }
    tiles
}

/// Render size by directory ownership, color by top owner, with full title labels.
pub(super) fn render(authors: &[&Author], report: &Report, options: Options) -> String {
    let mut directories: Vec<_> = report
        .directories
        .iter()
        .filter(|d| d.total_lines > 0.0)
        .collect();
    directories.sort_by(|a, b| {
        b.total_lines
            .total_cmp(&a.total_lines)
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.owner.cmp(&b.owner))
            .then_with(|| a.owner_share.total_cmp(&b.owner_share))
    });
    let mut out = canvas(options, 360, "Directory ownership treemap");
    let (bg, fg, _) = options.theme.colors();
    text(&mut out, 16, 24, "Directory ownership (area: lines)", fg);
    let maximum = directories.first().map_or(0.0, |d| d.total_lines);
    if maximum == 0.0 {
        text(&mut out, 16, 54, "no data", fg);
    } else {
        // Normalize before summing to avoid overflowing even with extreme finite input.
        let total: f64 = directories.iter().map(|d| d.total_lines / maximum).sum();
        let width = f64::from(options.width - 32);
        let areas: Vec<_> = directories
            .iter()
            .map(|d| (d.total_lines / maximum) / total * width * 300.0)
            .collect();
        let tiles = squarify(
            &areas,
            Rect {
                x: 16.0,
                y: 42.0,
                width,
                height: 300.0,
            },
        );
        for (directory, tile) in directories.iter().zip(tiles) {
            let path = if directory.path == "." {
                "(root)"
            } else {
                &directory.path
            };
            let rank = authors
                .iter()
                .position(|a| a.name == directory.owner)
                .unwrap_or(7);
            let color = options.theme.color(rank);
            let label = format!(
                "{}: {} {:.1}% ({:.1} lines)",
                path,
                directory.owner,
                directory.owner_share * 100.0,
                directory.total_lines
            );
            let _ = write!(
                out,
                "<g><title>{}</title><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{color}\" stroke=\"{bg}\" stroke-width=\"1\"/>",
                xml(&label),
                tile.x,
                tile.y,
                tile.width,
                tile.height
            );
            let owner = format!("{} {:.1}%", directory.owner, directory.owner_share * 100.0);
            let label_width = measure(path).max(measure(&owner));
            if tile.width >= f64::from(label_width + 12) && tile.height >= 44.0 {
                let _ = write!(
                    out,
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"36\" fill=\"{bg}\"/>",
                    tile.x + 3.0,
                    tile.y + 3.0,
                    label_width + 6
                );
                // Text y is integral in the shared helper; a translation retains precision.
                let _ = write!(
                    out,
                    "<g transform=\"translate({} {})\">",
                    tile.x + 6.0,
                    tile.y + 3.0
                );
                text(&mut out, 0, 13, path, fg);
                text(&mut out, 0, 29, &owner, fg);
                out.push_str("</g>");
            }
            out.push_str("</g>");
        }
    }
    out.push_str("</svg>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Squarification conserves total area and stays inside its canvas.
    #[test]
    fn layout_conserves_area_and_is_bounded() {
        let areas = [4500.0, 2500.0, 2000.0, 999.999, 0.001];
        let tiles = squarify(
            &areas,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
        );
        for (tile, area) in tiles.iter().zip(areas) {
            assert!((tile.width * tile.height - area).abs() < 0.000_001);
            assert!(tile.x >= 0.0 && tile.y >= 0.0);
            assert!(tile.x + tile.width <= 100.000_001);
            assert!(tile.y + tile.height <= 100.000_001);
        }
    }
}
