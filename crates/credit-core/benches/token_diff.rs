//! Reproducible diff-only timings on pre-interned token streams.
//! Run with `cargo bench -p credit-core --bench token_diff --locked`.

use std::{hint::black_box, time::Instant};

/// Compare fixed workloads and iteration counts, without algorithm deadlines.
fn main() {
    let ordinary: Vec<u32> = (0..12_000).collect();
    let repetitive: Vec<u32> = (0..12_000).map(|i| i % 7).collect();
    let generated: Vec<u32> = (0..30_000)
        .map(|i| if i % 11 == 0 { i + 100 } else { i % 11 })
        .collect();
    let rewrite: Vec<u32> = (0..8_000).collect();
    for (name, old, new) in [
        ("ordinary", ordinary.clone(), edited(&ordinary, 101)),
        ("repetitive", repetitive.clone(), edited(&repetitive, 37)),
        ("generated", generated.clone(), edited(&generated, 11)),
        (
            "near-total rewrite",
            rewrite.clone(),
            rewrite
                .iter()
                .enumerate()
                .map(|(i, id)| if i % 1000 == 0 { *id } else { id + 40_000 })
                .collect(),
        ),
        (
            "disjoint rewrite",
            rewrite.clone(),
            rewrite.iter().map(|id| id + 40_000).collect(),
        ),
    ] {
        let before: Vec<_> = old.iter().copied().map(imara_diff::Token).collect();
        let after: Vec<_> = new.iter().copied().map(imara_diff::Token).collect();
        let vocabulary = old.iter().chain(&new).copied().max().unwrap_or(0) + 1;
        let start = Instant::now();
        for _ in 0..5 {
            black_box(similar::capture_diff_slices(
                similar::Algorithm::Myers,
                black_box(&old),
                black_box(&new),
            ));
        }
        let myers = start.elapsed().as_secs_f64() * 200.0;
        let start = Instant::now();
        for _ in 0..5 {
            let mut diff = imara_diff::Diff::default();
            diff.compute_with(
                imara_diff::Algorithm::Histogram,
                black_box(&before),
                black_box(&after),
                vocabulary,
            );
            black_box(diff);
        }
        let histogram = start.elapsed().as_secs_f64() * 200.0;
        println!(
            "{name}: {} tokens, Myers {myers:.3} ms, histogram {histogram:.3} ms",
            old.len()
        );
    }
}

/// Introduce evenly spaced replacements outside the source vocabulary.
fn edited(old: &[u32], stride: usize) -> Vec<u32> {
    old.iter()
        .enumerate()
        .map(|(i, &id)| if i % stride == 0 { id + 40_000 } else { id })
        .collect()
}
