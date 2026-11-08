//! Validated CLI/config precedence and built-in path exclusions.

use crate::Analyze;
use anyhow::{Context, bail};
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::Deserialize;

/// Lockfiles and generated paths are excluded at every directory depth.
const BUILT_INS: &[&str] = &[
    "**/Cargo.lock",
    "**/package-lock.json",
    "**/pnpm-lock.yaml",
    "**/yarn.lock",
    "**/poetry.lock",
    "**/Gemfile.lock",
    "**/composer.lock",
    "**/go.sum",
    "**/uv.lock",
    "**/bun.lockb",
    "**/vendor/**",
    "**/node_modules/**",
    "**/dist/**",
    "**/*.min.js",
    "**/*.min.css",
    "**/*.map",
];

/// Optional root configuration. Unknown keys fail rather than silently misconfigure.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    cache: Option<bool>,
    granularity: Option<String>,
    bulk_discount: Option<bool>,
    walk: Option<String>,
    keep: Option<f64>,
    #[serde(default)]
    exclude: Vec<String>,
    #[serde(default)]
    opt_out: Vec<String>,
    max_file_size: Option<u64>,
    group_by: Option<String>,
}
/// Validated effective policy, reused by analysis and output metadata.
pub(crate) struct Settings {
    pub(crate) options: credit_git::Options,
    pub(crate) excluded: GlobSet,
    pub(crate) patterns: Vec<String>,
}
impl Settings {
    /// Read optional config and apply only explicitly supplied CLI overrides.
    pub(crate) fn load(args: &Analyze) -> anyhow::Result<Self> {
        let path = credit_git::repository_root(&args.path)?.join(".git-credit.toml");
        let config: Config = match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).context("invalid .git-credit.toml")?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
            Err(e) => return Err(e).context("cannot read .git-credit.toml"),
        };
        let configured_granularity = match config.granularity.as_deref().unwrap_or("token") {
            "line" => credit_git::Granularity::Line,
            "token" => credit_git::Granularity::Token,
            _ => bail!("granularity must be 'line' or 'token'"),
        };
        let granularity = match args.granularity.as_deref() {
            Some("line") => credit_git::Granularity::Line,
            Some("token") => credit_git::Granularity::Token,
            _ => configured_granularity,
        };
        if args.keep.is_some() && granularity != credit_git::Granularity::Line {
            bail!("--keep applies only to --granularity line");
        }
        let keep = args
            .keep
            .map_or_else(
                || {
                    config
                        .keep
                        .map_or(Ok(700), |value| parse_keep(&value.to_string()))
                },
                Ok,
            )
            .map_err(anyhow::Error::msg)
            .context("invalid keep policy")?;
        let group_by = if args.by_email {
            credit_git::GroupBy::Email
        } else {
            match config.group_by.as_deref().unwrap_or("name") {
                "name" => credit_git::GroupBy::Name,
                "email" => credit_git::GroupBy::Email,
                _ => bail!("group_by must be 'name' or 'email'"),
            }
        };
        let configured_walk = match config.walk.as_deref().unwrap_or("full") {
            "full" => credit_git::Walk::Full,
            "first-parent" => credit_git::Walk::FirstParent,
            _ => bail!("walk must be 'full' or 'first-parent'"),
        };
        let custom = if args.exclude.is_empty() {
            config.exclude
        } else {
            args.exclude.clone()
        };
        let mut patterns: Vec<_> = BUILT_INS.iter().map(|s| (*s).to_owned()).collect();
        patterns.extend(custom);
        patterns.sort();
        patterns.dedup();
        let mut builder = GlobSetBuilder::new();
        for pattern in &patterns {
            builder.add(
                Glob::new(pattern).with_context(|| format!("invalid exclusion glob: {pattern}"))?,
            );
        }
        let max_file_size = config.max_file_size.unwrap_or(1_048_576);
        if max_file_size == 0 {
            bail!("max_file_size must be greater than zero");
        }
        Ok(Self {
            options: credit_git::Options {
                cache: (!args.no_cache && config.cache.unwrap_or(true)).then(|| {
                    credit_git::CacheOptions {
                        directory: args.cache_dir.clone(),
                        depth: args.cache_depth,
                        exclusions: patterns.clone(),
                    }
                }),
                granularity,
                bulk_discount: !args.no_bulk_discount && config.bulk_discount.unwrap_or(true),
                walk: if args.first_parent {
                    credit_git::Walk::FirstParent
                } else {
                    configured_walk
                },
                policy: credit_core::Policy {
                    keep_per_mille: keep,
                },
                max_file_size,
                depth: args.depth,
                opt_out: config.opt_out,
                group_by,
            },
            excluded: builder.build()?,
            patterns,
        })
    }
}

/// Validate finite bounds and quantize the CLI/config threshold to per-mille units.
pub(crate) fn parse_keep(text: &str) -> Result<u32, String> {
    let value: f64 = text
        .parse()
        .map_err(|_| "keep must be a number in 0.0..=1.0")?;
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err("keep must be a finite number in 0.0..=1.0".into());
    }
    format!("{:.0}", value * 1000.0)
        .parse()
        .map_err(|_| "invalid keep threshold".into())
}
/// Validate command-line globs early so invalid patterns have usage exit code 2.
pub(crate) fn parse_glob(text: &str) -> Result<String, String> {
    Glob::new(text).map_err(|e| e.to_string())?;
    Ok(text.to_owned())
}
