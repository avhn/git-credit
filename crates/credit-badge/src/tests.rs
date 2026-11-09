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
        ..Report::default()
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

/// Decode hexadecimal colors into WCAG relative luminance.
fn luminance(hex: &str) -> f64 {
    let hex = hex.trim_start_matches('#');
    let expanded = if hex.len() == 3 {
        hex.chars().flat_map(|c| [c, c]).collect::<String>()
    } else {
        hex.to_owned()
    };
    let value = u32::from_str_radix(&expanded, 16).unwrap();
    let channel = |shift| {
        let v = f64::from((value >> shift) & 255_u32) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
}

/// WCAG contrast applies to both chart fills and all theme text tokens.
#[test]
fn palette_and_text_pass_wcag_in_both_themes() {
    let contrast = |a: &str, b: &str| {
        let a = luminance(a);
        let b = luminance(b);
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    };
    for theme in [Theme::Light, Theme::Dark] {
        let (bg, fg, muted) = theme.colors();
        assert!(contrast(bg, fg) >= 4.5);
        assert!(contrast(bg, muted) >= 4.5);
        assert!(contrast(bg, theme.neutral()) >= 3.0);
        for rank in 0..PALETTE.len() {
            assert!(
                contrast(bg, theme.color(rank)) >= 3.0,
                "{theme:?} {}",
                theme.color(rank)
            );
        }
    }
}

/// New charts handle full rings, empty data, tiny shares and unsafe text identically.
#[test]
fn charts_are_proportional_safe_and_repeatable() {
    for theme in [Theme::Light, Theme::Dark] {
        let options = Options {
            theme,
            ..Options::default()
        };
        let mut input = Report {
            total_lines: 100.0,
            ..report()
        };
        input.authors[0].name = "<script>&\"\u{202e}\x1b[31m\n".into();
        input.directories = vec![Directory {
            path: "src<&".into(),
            total_lines: 100.0,
            owner: input.authors[0].name.clone(),
            owner_share: 0.5,
        }];
        let first = render(&input, options).unwrap();
        assert_eq!(first, render(&input, options).unwrap());
        for chart in [
            &first.bar,
            &first.card,
            &first.donut,
            &first.labor,
            &first.treemap,
        ] {
            check_xml(chart);
            assert!(!chart.contains('\u{202e}'));
            assert!(!chart.contains('\x1b'));
            assert!(!chart.contains("<script>"));
            assert!(!chart.contains("NaN"));
        }
        assert!(first.labor.contains("including later rewritten"));
        assert!(first.labor.contains("50.0"));
        assert!(first.treemap.contains("50.0%"));
        input.authors = vec![Author {
            name: "solo".into(),
            ownership_share: 1.0,
            labor: 1.0,
        }];
        let solo = render(&input, options).unwrap();
        assert!(solo.donut.contains("<circle"));
        assert!(solo.donut.contains("100.0%"));
        input.authors = vec![
            Author {
                name: "large".into(),
                ownership_share: 1.0 - 1e-9,
                labor: 1.0,
            },
            Author {
                name: "tiny".into(),
                ownership_share: 1e-9,
                labor: 1.0,
            },
        ];
        let tiny = render(&input, options).unwrap();
        assert_eq!(tiny.donut.matches("<path ").count(), 2);
        assert!(tiny.donut.contains("tiny: 0.0%"));
        let empty = render(&Report::default(), options).unwrap();
        for chart in [&empty.donut, &empty.labor, &empty.treemap] {
            check_xml(chart);
            assert!(chart.contains("no data"));
        }
    }
}

/// Grouped donut labels reuse exact tenths apportionment, not independent rounding.
#[test]
fn donut_legend_sums_to_one_hundred_and_groups_others() {
    let input = Report {
        authors: (0..6)
            .map(|i| Author {
                name: format!("Author {i}"),
                ownership_share: 1.0 / 6.0,
                labor: 1.0,
            })
            .collect(),
        ..Report::default()
    };
    let assets = render(
        &input,
        Options {
            top: 2,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(assets.donut.contains("Author 0: 16.7%"));
    let total: f64 = input.authors.iter().map(|a| a.ownership_share).sum();
    let others: f64 = input
        .authors
        .iter()
        .skip(2)
        .map(|a| a.ownership_share)
        .sum();
    let units = percentages(&[1.0 / 6.0, 1.0 / 6.0, others], total);
    assert_eq!(units.iter().sum::<u32>(), 1000);
    for (name, unit) in ["Author 0", "Author 1", "Others"].into_iter().zip(units) {
        assert!(
            assets
                .donut
                .contains(&format!("{name}: {}.{}%", unit / 10, unit % 10))
        );
    }
}

/// Reserve theme suffixes before producing per-author light/dark asset paths.
#[test]
fn author_stems_cannot_collide_with_dark_companions() {
    let input = Report {
        authors: [
            "Alice",
            "Alice Dark",
            "Alice Dark Author",
            "Alice",
            "Alice Dark 2",
        ]
        .into_iter()
        .map(|name| Author {
            name: name.into(),
            ownership_share: 0.0,
            labor: 1.0,
        })
        .collect(),
        ..Report::default()
    };
    let assets = render(&input, Options::default()).unwrap();
    let mut paths = std::collections::BTreeSet::new();
    for (stem, _) in assets.authors {
        assert!(paths.insert(format!("{stem}.svg")));
        assert!(paths.insert(format!("{stem}-dark.svg")));
    }
}

/// Numeric labels are not truncated, even at the smallest supported chart width.
#[test]
fn labor_values_and_explanations_fit_narrow_charts() {
    let input = Report {
        authors: vec![Author {
            name: "Alice".into(),
            ownership_share: 1.0,
            labor: 13424.6,
        }],
        total_lines: 13045.0,
        ..Report::default()
    };
    let chart = render(
        &input,
        Options {
            width: 200,
            ..Options::default()
        },
    )
    .unwrap()
    .labor;
    assert!(chart.contains(">13424.6</text>"));
    assert!(chart.contains(">13045.0</text>"));
    assert!(!chart.contains("..."));
    for line in charts::wrap("Labor: all meaningful work, including later rewritten", 168) {
        assert!(measure(&line) <= 168);
    }
}

/// Directory input order is irrelevant and invalid geometry is rejected before SVG output.
#[test]
fn directory_rendering_is_order_independent_and_validated() {
    let mut input = Report {
        total_lines: 2.0,
        ..report()
    };
    input.directories = ["z", "a", "b"]
        .into_iter()
        .map(|path| Directory {
            path: path.into(),
            total_lines: 1.0,
            owner: "Zoë".into(),
            owner_share: 0.7,
        })
        .collect();
    let first = render(&input, Options::default()).unwrap();
    input.directories.reverse();
    assert_eq!(first, render(&input, Options::default()).unwrap());
    for invalid in [f64::NAN, f64::INFINITY, -1.0] {
        input.directories[0].total_lines = invalid;
        assert_eq!(
            render(&input, Options::default()),
            Err(Error::InvalidDirectory)
        );
    }
}

/// Aggregates use the theme's neutral gray for both geometry and legend swatches.
#[test]
fn others_uses_neutral_gray_in_both_themes() {
    let mut input = report();
    // An actual author named Others must retain the author palette color.
    input.authors[0].name = "Others".into();
    for theme in [Theme::Light, Theme::Dark] {
        let assets = render(
            &input,
            Options {
                top: 1,
                theme,
                ..Options::default()
            },
        )
        .unwrap();
        let neutral = theme.neutral();
        assert_eq!(
            assets.bar.matches(&format!("fill=\"{neutral}\"")).count(),
            2
        );
        assert_eq!(
            assets
                .donut
                .matches(&format!("stroke=\"{neutral}\""))
                .count(),
            1
        );
        assert_eq!(
            assets.donut.matches(&format!("fill=\"{neutral}\"")).count(),
            1
        );
        assert!(
            assets
                .donut
                .contains(&format!("stroke=\"{}\"", theme.color(0)))
        );
        assert!(assets.bar.contains("others 50.0%"));
        assert!(assets.donut.contains("Others: 50.0%"));
    }
}

/// Root naming is presentation-only and applies to both tooltip and visible label.
#[test]
fn treemap_displays_repository_root_explicitly() {
    let input = Report {
        directories: vec![Directory {
            path: ".".into(),
            total_lines: 10.0,
            owner: "Alice".into(),
            owner_share: 1.0,
        }],
        ..Report::default()
    };
    for theme in [Theme::Light, Theme::Dark] {
        let chart = render(
            &input,
            Options {
                theme,
                ..Options::default()
            },
        )
        .unwrap()
        .treemap;
        assert!(chart.contains("<title>(root): Alice 100.0%"));
        assert!(chart.contains(">(root)</text>"));
        assert!(!chart.contains(">.</text>"));
    }
    assert_eq!(input.directories[0].path, ".");
}
