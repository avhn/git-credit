//! Snapshot-free invariants for the small SVG vocabulary emitted by this crate.

use super::*;

/// Build ranked data with a malicious name, quotes, ampersands and RTL Unicode.
fn report() -> Report {
    Report {
        authors: vec![
            Author {
                name: "<script> & \"Alice\" 'שלום'".into(),
                ownership_share: 0.5,
                labor: 10.25,
            },
            Author {
                name: "Zoë".into(),
                ownership_share: 0.3,
                labor: 3.0,
            },
            Author {
                name: "李明".into(),
                ownership_share: 0.2,
                labor: 8.0,
            },
        ],
        commits: 12,
        head: "0123456789abcdef".into(),
    }
}

/// Tiny checker for balanced tags, quoted attributes and entity-only ampersands.
/// It intentionally accepts only the generated subset, not arbitrary XML.
fn check_xml(svg: &str) {
    assert!(svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\" version=\"1.1\""));
    let mut stack = Vec::new();
    let mut rest = svg;
    while let Some(start) = rest.find('<') {
        let before = &rest[..start];
        assert!(!before.contains('>'));
        let end = rest[start..].find('>').unwrap() + start;
        let tag = &rest[start + 1..end];
        assert!(!tag.contains('<'));
        assert_eq!(tag.matches('"').count() % 2, 0);
        if let Some(name) = tag.strip_prefix('/') {
            assert_eq!(stack.pop(), Some(name));
        } else if !tag.ends_with('/') {
            stack.push(tag.split_whitespace().next().unwrap());
        }
        rest = &rest[end + 1..];
    }
    assert_eq!(stack, [] as [&str; 0]);
    for tail in svg.split('&').skip(1) {
        assert!(
            ["amp;", "lt;", "gt;", "quot;", "apos;"]
                .iter()
                .any(|e| tail.starts_with(e))
        );
    }
    assert!(svg.chars().all(|c| c >= ' ' || c == '\n'));
}

#[test]
fn all_assets_are_valid_escaped_and_deterministic() {
    let input = report();
    let first = render(&input, Options::default()).unwrap();
    assert_eq!(first, render(&input, Options::default()).unwrap());
    for svg in [&first.bar, &first.card, &first.bus_factor]
        .into_iter()
        .chain(first.authors.iter().map(|(_, svg)| svg))
    {
        check_xml(svg);
        assert!(!svg.contains("<script>"));
    }
    assert!(
        first
            .card
            .contains("&lt;script&gt; &amp; &quot;Alice&quot; &apos;שלום&apos;")
    );
    assert!(first.card.contains("Zoë"));
    assert!(first.card.contains("李明"));
    assert!(first.card.contains("HEAD 0123456789ab"));
    let mut reversed = input;
    reversed.authors.reverse();
    assert_eq!(first, render(&reversed, Options::default()).unwrap());
}

#[test]
fn rounding_is_complete_and_others_are_in_denominator() {
    assert_eq!(percentages(&[1.0 / 3.0; 3], 1.0), [334, 333, 333]);
    let badges = render(
        &report(),
        Options {
            top: 1,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(badges.bar.contains("50.0%"));
    assert!(badges.bar.contains("others 50.0%"));
    assert_eq!(badges.authors.len(), 1);
    assert!(badges.authors[0].1.contains("50.0% owned"));
    assert!(badges.bus_factor_json.contains("\"message\":\"2\""));
}

#[test]
fn strict_majority_and_semantic_colors() {
    for (shares, factor, color) in [
        (vec![0.51, 0.49], 1, "#c62828"),
        (vec![0.5, 0.5], 2, "#9c6500"),
        (vec![0.25; 4], 3, "#237a3b"),
        (vec![0.2, 0.2, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1], 4, "#237a3b"),
    ] {
        let input = Report {
            authors: shares
                .into_iter()
                .enumerate()
                .map(|(i, ownership_share)| Author {
                    name: i.to_string(),
                    ownership_share,
                    labor: 1.0,
                })
                .collect(),
            ..Report::default()
        };
        let assets = render(&input, Options::default()).unwrap();
        assert!(
            assets
                .bus_factor_json
                .contains(&format!("\"message\":\"{factor}\""))
        );
        assert!(assets.bus_factor.contains(color));
    }
}

#[test]
fn empty_and_labor_only_reports_do_not_invent_ownership() {
    for input in [
        Report::default(),
        Report {
            authors: vec![Author {
                name: "Alice".into(),
                ownership_share: 0.0,
                labor: 5.0,
            }],
            ..Report::default()
        },
    ] {
        let assets = render(&input, Options::default()).unwrap();
        for svg in [&assets.bar, &assets.card, &assets.bus_factor] {
            check_xml(svg);
        }
        assert!(assets.bar.contains("no data"));
        assert!(assets.bus_factor_json.contains("no data"));
        assert!(assets.top_contributor_json.contains("no data"));
    }
}

#[test]
fn input_validation_and_xml_forbidden_characters() {
    assert_eq!(
        render(
            &report(),
            Options {
                top: 0,
                ..Options::default()
            }
        ),
        Err(Error::InvalidTop)
    );
    assert_eq!(
        render(
            &report(),
            Options {
                width: 0,
                ..Options::default()
            }
        ),
        Err(Error::InvalidWidth)
    );
    for value in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
        let mut input = report();
        input.authors[0].ownership_share = value;
        assert_eq!(
            render(&input, Options::default()),
            Err(Error::InvalidAuthor)
        );
    }
    let mut input = report();
    input.authors[0].ownership_share = 0.2;
    assert_eq!(
        render(&input, Options::default()),
        Err(Error::InvalidShareTotal)
    );
    input = report();
    input.authors[0].labor = -1.0;
    assert_eq!(
        render(&input, Options::default()),
        Err(Error::InvalidAuthor)
    );
    assert_eq!(xml("\0\u{fffe}\u{ffff}"), "\u{fffd}\u{fffd}\u{fffd}");
    assert_eq!(json_string("\"\\\n\0"), "\"\\\"\\\\\\u000a\\u0000\"");
}

#[test]
fn filename_collisions_devices_and_traversal_are_safe() {
    let input = Report {
        authors: ["../CON", "../CON", "שלום", "שלום", "a/b", "a b", "a-b-2"]
            .into_iter()
            .map(|name| Author {
                name: name.into(),
                ownership_share: 0.0,
                labor: 1.0,
            })
            .collect(),
        ..Report::default()
    };
    let assets = render(
        &input,
        Options {
            top: 10,
            ..Options::default()
        },
    )
    .unwrap();
    let stems: std::collections::BTreeSet<_> =
        assets.authors.iter().map(|(stem, _)| stem).collect();
    assert_eq!(stems.len(), input.authors.len());
    for stem in stems {
        assert!(stem.starts_with("author-"));
        assert!(
            stem.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        );
    }
}

#[test]
fn metrics_fit_columns_and_themes_remain_legible() {
    assert!(measure("WWW") > measure("iii"));
    assert!(measure(&fit(&"W".repeat(500), 100)) <= 100);
    let dark = render(
        &report(),
        Options {
            theme: Theme::Dark,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(dark.card.contains("#161b22"));
    assert!(dark.card.contains("#f0f6fc"));
    assert_ne!(
        dark.card,
        render(&report(), Options::default()).unwrap().card
    );
}
