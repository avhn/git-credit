//! Merge-aware object replay and live-tree aggregation.

use crate::scoring::{self, Change, State};
use crate::{Directory, Error, Options, Report, git_error, identities::Authors};
use credit_core::{ChangeKind, Ledger};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::Arc,
};

/// Analyze committed HEAD offline, applying `excluded` to repository-relative
/// slash-separated paths before scoring. The callback returns true to skip a path.
/// Shallow repositories fail before scoring. Binary blobs, oversized blobs,
/// symlinks and submodules are not scored. Missing objects are errors, not roots.
///
/// # Errors
/// Returns typed discovery, empty/shallow history, object, identity or core errors.
pub fn analyze(
    path: &Path,
    options: &Options,
    excluded: &dyn Fn(&str) -> bool,
) -> Result<Report, Error> {
    let mut resumed = false;
    match analyze_attempt(path, options, excluded, true, &mut resumed) {
        Err(Error::Core(error)) if resumed => {
            let mut report = analyze_attempt(path, options, excluded, false, &mut false)?;
            report.cache_diagnostics.insert(
                0,
                format!("Cache resumed replay failed: {error}; retrying cold without cached state"),
            );
            Ok(report)
        }
        result => result,
    }
}

/// Perform one isolated attempt. Disable cache reads on retry, but publish a
/// rebuilt generation only after replay and all report accounting have succeeded.
fn analyze_attempt(
    path: &Path,
    options: &Options,
    excluded: &dyn Fn(&str) -> bool,
    read_cache: bool,
    resumed: &mut bool,
) -> Result<Report, Error> {
    options.policy.validate()?;
    let repo = gix::discover(path).map_err(git_error)?;
    if repo.is_shallow().map_err(git_error)? {
        return Err(Error::ShallowRepository);
    }
    if repo.head().map_err(git_error)?.is_unborn() {
        return Err(Error::EmptyRepository);
    }
    let head = repo.head_commit().map_err(git_error)?;
    let mut authors = Authors::load(&repo, &head)?;
    let mut history = crate::history::History::load(&repo, head.id, options.walk)?;
    // Resolve identities in cold-walk order: deduplication affects rounding.
    let mut identities = BTreeMap::new();
    for id in &history.order {
        identities.insert(
            id.to_string(),
            authors.commit(&repo.find_commit(*id).map_err(git_error)?, options)?,
        );
    }
    let groups = authors.keys();
    let mut diagnostics = Vec::new();
    let directory = options.cache.as_ref().map(|cache| {
        cache
            .directory
            .clone()
            .unwrap_or_else(|| repo.git_dir().join("git-credit/cache"))
    });
    let (pack, retained) = prepare_cache(
        &repo,
        directory.as_deref().filter(|_| read_cache),
        &history,
        &identities,
        &groups,
        options,
        &mut diagnostics,
    )?;
    *resumed = !pack.commits.is_empty();
    let wanted = snapshot_ids(
        &history,
        head.id,
        options.cache.as_ref().map_or(0, |cache| cache.depth),
    );
    let snapshots: BTreeMap<_, _> = retained
        .iter()
        .filter(|(id, _)| wanted.contains(*id))
        .map(|(id, files)| (*id, files.clone()))
        .collect();
    let state = Replay::default();
    let mut run = ReplayRun {
        pack,
        retained,
        snapshots,
        state,
        peak: 0,
        replayed: 0,
    };
    run.execute(&repo, &mut history, &identities, options, excluded, &wanted)?;
    let ReplayRun {
        mut pack,
        mut retained,
        snapshots,
        mut state,
        peak: peak_retained_states,
        replayed,
    } = run;
    state.files = retained
        .remove(&head.id)
        .ok_or_else(|| Error::InvalidInput("missing HEAD state".into()))?;
    let bulk_edit_commits = aggregate(&mut state, &pack, &history)?;
    pack.identity_key = crate::cache::identity_key(pack.commits.keys(), &identities, &groups)?;
    pack.groups = groups;
    state.ledger.ownership = scoring::ownership(state.files.values().map(|file| &file.state))?;
    let mut directories = directories(&state.files, options.depth)?;
    let authors = authors.finish(options, &mut state.ledger, &mut directories)?;
    let commits_analyzed = u64::try_from(history.order.len())
        .map_err(|_| Error::InvalidInput("too many commits".into()))?;
    persist_cache(
        directory.as_deref(),
        &mut pack,
        head.id,
        &snapshots,
        replayed,
        &mut diagnostics,
    );
    Ok(Report {
        cache_diagnostics: diagnostics,
        granularity: options.granularity,
        bulk_edit_commits,
        walk: options.walk,
        peak_retained_states,
        head: head.id.to_string(),
        commits_analyzed,
        formatting_commits: state.formatting,
        authors,
        ledger: state.ledger,
        directories,
        warnings: state.warnings.into_iter().collect(),
    })
}

/// Prepare an empty generation or a fully validated resumable generation.
fn prepare_cache(
    repo: &gix::Repository,
    directory: Option<&Path>,
    history: &crate::history::History,
    identities: &BTreeMap<String, credit_core::LineOwner>,
    groups: &BTreeMap<u32, String>,
    options: &Options,
    diagnostics: &mut Vec<String>,
) -> Result<(crate::cache::Pack, BTreeMap<gix::ObjectId, Files>), Error> {
    let key = crate::cache::key(options)?;
    if let Some(directory) = directory {
        match resume(repo, directory, &key, history, identities, groups, options) {
            Ok((pack, snapshots)) => {
                diagnostics.push(format!(
                    "Cache resumed: {} processed commits",
                    pack.commits.len()
                ));
                return Ok((pack, snapshots));
            }
            Err(error) => diagnostics.push(format!("Cache cold replay: {error}")),
        }
    }
    Ok((
        crate::cache::Pack {
            key,
            ..Default::default()
        },
        BTreeMap::new(),
    ))
}

/// Report optional cache write failures without changing analysis output.
fn persist_cache(
    directory: Option<&Path>,
    pack: &mut crate::cache::Pack,
    head: gix::ObjectId,
    snapshots: &BTreeMap<gix::ObjectId, Files>,
    replayed: usize,
    diagnostics: &mut Vec<String>,
) {
    if let Some(directory) = directory {
        pack.head = head.to_string();
        match save_snapshots(directory, pack, snapshots) {
            Ok(bytes) => diagnostics.push(format!("Cache replayed {replayed} new commits; {bytes} bytes (limit 268435456); {} snapshots", snapshots.len())),
            Err(error) => diagnostics.push(format!("Cache not written: {error}")),
        }
    }
}

/// Mutable execution state shared across the ordered commit replay loop.
struct ReplayRun {
    pack: crate::cache::Pack,
    retained: BTreeMap<gix::ObjectId, Files>,
    snapshots: BTreeMap<gix::ObjectId, Files>,
    state: Replay,
    peak: usize,
    replayed: usize,
}
impl ReplayRun {
    /// Replay only unprocessed commits after all cache boundaries are validated.
    fn execute(
        &mut self,
        repo: &gix::Repository,
        history: &mut crate::history::History,
        identities: &BTreeMap<String, credit_core::LineOwner>,
        options: &Options,
        excluded: &dyn Fn(&str) -> bool,
        wanted: &BTreeSet<gix::ObjectId>,
    ) -> Result<(), Error> {
        let mut cache = raw_diff_cache(repo.object_hash(), options.max_file_size);
        for id in &history.order {
            if self.pack.commits.contains_key(&id.to_string()) {
                continue;
            }
            let commit = repo.find_commit(*id).map_err(git_error)?;
            let shares = identities
                .get(&id.to_string())
                .ok_or_else(|| Error::InvalidInput("missing commit identity".into()))?;
            let tree = commit.tree().map_err(git_error)?;
            let parents = history
                .parents
                .get(id)
                .ok_or_else(|| Error::InvalidInput("missing scheduled commit".into()))?;
            let previous = if let Some(parent) = parents.first() {
                self.state.files = self
                    .retained
                    .get(parent)
                    .ok_or_else(|| Error::InvalidInput("missing parent state".into()))?
                    .clone();
                repo.find_commit(*parent)
                    .map_err(git_error)?
                    .tree()
                    .map_err(git_error)?
            } else {
                self.state.files = BTreeMap::new();
                repo.empty_tree()
            };
            let changes = tree_changes(&previous, &tree, &mut cache)?;
            cache.clear_resource_cache();
            let other_parents = merge_parents(repo, parents, &self.retained, &tree, &mut cache)?;
            let bulk = self.state.commit(
                repo,
                changes,
                shares.authors(),
                options,
                excluded,
                &other_parents,
            )?;
            if options.walk == crate::Walk::FirstParent && commit.parent_ids().count() > 1 {
                self.state.warnings.insert("Merge attribution is approximate: first-parent merge diffs credit the merge author and co-authors.".into());
            }
            self.pack.commits.insert(
                id.to_string(),
                crate::cache::Commit {
                    identity: crate::cache::shares(shares),
                    labor: std::mem::take(&mut self.state.ledger.labor)
                        .into_iter()
                        .map(|(id, amount)| (id.0, amount))
                        .collect(),
                    formatting: self.state.formatting > 0,
                    bulk,
                    warnings: std::mem::take(&mut self.state.warnings)
                        .into_iter()
                        .collect(),
                },
            );
            self.state.formatting = 0;
            self.replayed += 1;
            self.peak = self.peak.max(self.retained.len() + 1);
            for parent in parents {
                if let Some(count) = history.remaining_children.get_mut(parent) {
                    *count -= 1;
                    if *count == 0 {
                        self.retained.remove(parent);
                    }
                }
            }
            if wanted.contains(id) {
                self.snapshots.insert(*id, self.state.files.clone());
            }
            self.retained
                .insert(*id, std::mem::take(&mut self.state.files));
        }
        Ok(())
    }
}

/// Fold commit accounting in cold replay order, including bounded bulk examples.
fn aggregate(
    state: &mut Replay,
    pack: &crate::cache::Pack,
    history: &crate::history::History,
) -> Result<u64, Error> {
    let mut bulk_edit_commits = 0;
    let mut bulk_examples = Vec::new();
    for id in &history.order {
        let record = pack
            .commits
            .get(&id.to_string())
            .ok_or_else(|| Error::InvalidInput("missing commit accounting".into()))?;
        state.ledger.add_labor(
            &record
                .labor
                .iter()
                .map(|(id, amount)| (credit_core::AuthorId(*id), *amount))
                .collect(),
        )?;
        state.formatting += u64::from(record.formatting);
        state.warnings.extend(record.warnings.iter().cloned());
        if record.bulk {
            bulk_edit_commits += 1;
            if bulk_examples.len() < 5 {
                bulk_examples.push(id.to_string());
            }
        }
    }
    if !bulk_examples.is_empty() {
        state.warnings.insert(format!(
            "Bulk edits discounted in {bulk_edit_commits} commits; examples: {}",
            bulk_examples.join(", ")
        ));
    }
    Ok(bulk_edit_commits)
}

/// Retain HEAD and its first-parent chain, independent of full-walk order.
fn snapshot_ids(
    history: &crate::history::History,
    head: gix::ObjectId,
    depth: usize,
) -> BTreeSet<gix::ObjectId> {
    let mut result = BTreeSet::new();
    let mut next = Some(head);
    while let Some(id) = next {
        result.insert(id);
        if result.len() > depth {
            break;
        }
        next = history
            .parents
            .get(&id)
            .and_then(|parents| parents.first())
            .copied();
    }
    result
}

/// Validate processed ancestry, identities and resume boundaries before exposing
/// any cached state. Failure leaves the caller free to perform a cold replay.
fn resume(
    repo: &gix::Repository,
    directory: &Path,
    key: &str,
    history: &crate::history::History,
    identities: &BTreeMap<String, credit_core::LineOwner>,
    groups: &BTreeMap<u32, String>,
    options: &Options,
) -> Result<(crate::cache::Pack, BTreeMap<gix::ObjectId, Files>), Error> {
    let mut pack = crate::cache::load(directory)?;
    let invalid = |message: &str| Error::InvalidInput(message.to_owned());
    if pack.key != key {
        return Err(invalid("cache version or attribution policy changed"));
    }
    let head = gix::ObjectId::from_hex(pack.head.as_bytes()).map_err(git_error)?;
    if !history.parents.contains_key(&head) {
        return Err(invalid("cached HEAD is not an ancestor (history rewrite)"));
    }
    let mut ancestors = BTreeSet::new();
    let mut pending = vec![head];
    while let Some(id) = pending.pop() {
        if ancestors.insert(id.to_string()) {
            pending.extend(history.parents.get(&id).into_iter().flatten().copied());
        }
    }
    if !ancestors.iter().eq(pack.commits.keys()) {
        return Err(invalid(
            "processed commit set does not match cached ancestry",
        ));
    }
    if pack.identity_key != crate::cache::identity_key(pack.commits.keys(), identities, groups)? {
        return Err(invalid("identity digest changed (mailmap or grouping)"));
    }
    crate::cache::remap_identities(&mut pack, groups)?;
    let author_count = identities
        .values()
        .flat_map(credit_core::LineOwner::authors)
        .map(|(id, _)| id.0 as usize + 1)
        .max()
        .unwrap_or(0);
    let mut cached_labor = Ledger::default();
    for (id, record) in &pack.commits {
        cached_labor.add_labor(
            &record
                .labor
                .iter()
                .map(|(id, amount)| (credit_core::AuthorId(*id), *amount))
                .collect(),
        )?;
        if identities.get(id).map(crate::cache::shares).as_ref() != Some(&record.identity) {
            return Err(invalid(
                "identity digest changed (mailmap, grouping or replay identity order)",
            ));
        }
        if record.labor.keys().any(|id| *id as usize >= author_count) {
            return Err(invalid("cached labor has an invalid author"));
        }
    }
    validate_boundaries(&pack, history, &ancestors, options)?;
    if !pack.snapshots.contains_key(&pack.head) {
        return Err(invalid("missing cached HEAD snapshot"));
    }
    let mut retained = BTreeMap::new();
    let mut shared = BTreeMap::new();
    for (id, snapshot) in &pack.snapshots {
        if !ancestors.contains(id) {
            return Err(invalid("snapshot outside processed ancestry"));
        }
        let id = gix::ObjectId::from_hex(id.as_bytes()).map_err(git_error)?;
        let mut files = BTreeMap::new();
        for (path, hash) in snapshot {
            let key = (
                hash.clone(),
                crate::normalization(path) == credit_core::NormalizationMode::IgnoreIndentation,
            );
            let file = if let Some(file) = shared.get(&key) {
                Arc::clone(file)
            } else {
                let (id, state) =
                    crate::cache::restore_file(repo, &pack, hash, path, options, author_count)?;
                let file = Arc::new(TrackedFile { id, state });
                shared.insert(key, Arc::clone(&file));
                file
            };
            files.insert(path.clone(), file);
        }
        retained.insert(id, files);
    }
    Ok((pack, retained))
}

/// Require every old replay boundary and requested retained ancestor snapshot.
fn validate_boundaries(
    pack: &crate::cache::Pack,
    history: &crate::history::History,
    ancestors: &BTreeSet<String>,
    options: &Options,
) -> Result<(), Error> {
    let invalid = |message: &str| Error::InvalidInput(message.to_owned());
    for id in &history.order {
        if ancestors.contains(&id.to_string()) {
            continue;
        }
        for parent in history.parents.get(id).into_iter().flatten() {
            if ancestors.contains(&parent.to_string())
                && !pack.snapshots.contains_key(&parent.to_string())
            {
                return Err(invalid(
                    "required processed parent is outside cached snapshot depth",
                ));
            }
        }
    }
    if let Some(head) = history.order.last() {
        let wanted = snapshot_ids(
            history,
            *head,
            options.cache.as_ref().map_or(0, |cache| cache.depth),
        );
        if wanted.iter().any(|id| {
            ancestors.contains(&id.to_string()) && !pack.snapshots.contains_key(&id.to_string())
        }) {
            return Err(invalid(
                "requested cache depth requires older snapshots; rebuilding",
            ));
        }
    }
    Ok(())
}

/// Replace the generation with selected snapshots and only their reachable files.
fn save_snapshots(
    directory: &Path,
    pack: &mut crate::cache::Pack,
    snapshots: &BTreeMap<gix::ObjectId, Files>,
) -> Result<usize, Error> {
    pack.snapshots.clear();
    pack.files.clear();
    let mut encoded = BTreeMap::new();
    for (id, files) in snapshots {
        let mut snapshot = BTreeMap::new();
        for (path, file) in files {
            let pointer = Arc::as_ptr(file);
            let hash = if let Some(hash) = encoded.get(&pointer) {
                String::clone(hash)
            } else {
                let hash = crate::cache::store_file(pack, file.id, &file.state)?;
                encoded.insert(pointer, hash.clone());
                hash
            };
            snapshot.insert(path.clone(), hash);
        }
        pack.snapshots.insert(id.to_string(), snapshot);
    }
    crate::cache::save(directory, pack)
}

/// Build other-parent views, including rename sources, without copying file maps.
fn merge_parents<'a>(
    repo: &gix::Repository,
    parents: &[gix::ObjectId],
    retained: &'a BTreeMap<gix::ObjectId, Files>,
    tree: &gix::Tree<'_>,
    cache: &mut gix::diff::blob::Platform,
) -> Result<Vec<Parent<'a>>, Error> {
    let mut other_parents = Vec::new();
    for parent in parents.iter().skip(1) {
        let files = retained
            .get(parent)
            .ok_or_else(|| Error::InvalidInput("missing merge parent state".into()))?;
        let parent_tree = repo
            .find_commit(*parent)
            .map_err(git_error)?
            .tree()
            .map_err(git_error)?;
        let changes = tree_changes(&parent_tree, tree, cache)?;
        let renames = changes
            .iter()
            .filter_map(|change| {
                change
                    .source
                    .as_ref()
                    .map(|source| (change.path.clone(), source.clone()))
            })
            .collect();
        cache.clear_resource_cache();
        other_parents.push(Parent {
            files,
            changes,
            renames,
        });
    }
    Ok(other_parents)
}

/// Construct a raw-object-only cache once, without consulting repository config,
/// index entries, global/info attributes, worktree paths, drivers or textconv.
fn raw_diff_cache(hash: gix::hash::Kind, max_file_size: u64) -> gix::diff::blob::Platform {
    use gix::{
        diff::blob,
        worktree::stack::{
            State,
            state::{Attributes, attributes::Source},
        },
    };
    let attributes = Attributes::new(
        gix::attrs::Search::default(),
        None,
        Source::IdMapping,
        gix::attrs::search::MetadataCollection::default(),
    );
    let stack = gix::worktree::Stack::new(
        std::path::PathBuf::new(),
        State::AttributesStack(attributes),
        gix::glob::pattern::Case::Sensitive,
        Vec::new(),
        Vec::new(),
    );
    let filter = blob::Pipeline::new(
        blob::pipeline::WorktreeRoots::default(),
        gix::filter::plumbing::Pipeline::new(
            gix::command::Context::default(),
            hash,
            gix::filter::plumbing::pipeline::Options::default(),
        ),
        Vec::new(),
        blob::pipeline::Options {
            large_file_threshold_bytes: max_file_size.max(1),
            ..Default::default()
        },
    );
    blob::Platform::new(
        blob::platform::Options::default(),
        filter,
        blob::pipeline::Mode::ToGit,
        stack,
    )
}

/// A changed file or rename, detached from gix callback lifetimes.
struct FileChange {
    path: Vec<u8>,
    source: Option<Vec<u8>>,
    id: Option<gix::ObjectId>,
}

/// Enumerate changes using gix's deterministic exact and similarity rename tracking.
fn tree_changes(
    old: &gix::Tree<'_>,
    new: &gix::Tree<'_>,
    cache: &mut gix::diff::blob::Platform,
) -> Result<Vec<FileChange>, Error> {
    use gix::object::tree::diff::Change;
    let mut result = Vec::new();
    old.changes()
        .map_err(git_error)?
        .options(|options| {
            options
                .track_path()
                .track_rewrites(Some(gix::diff::Rewrites::default()));
        })
        .for_each_to_obtain_tree_with_cache(new, cache, |change| {
            let item = match change {
                Change::Addition {
                    location,
                    entry_mode,
                    id,
                    ..
                } if entry_mode.is_blob() => Some(FileChange {
                    path: location.to_vec(),
                    source: None,
                    id: Some(id.detach()),
                }),
                Change::Deletion {
                    location,
                    entry_mode,
                    ..
                } if entry_mode.is_blob() => Some(FileChange {
                    path: location.to_vec(),
                    source: None,
                    id: None,
                }),
                Change::Modification {
                    location,
                    entry_mode,
                    id,
                    previous_entry_mode,
                    ..
                } if entry_mode.is_blob() || previous_entry_mode.is_blob() => Some(FileChange {
                    path: location.to_vec(),
                    source: None,
                    id: entry_mode.is_blob().then(|| id.detach()),
                }),
                Change::Rewrite {
                    location,
                    source_location,
                    entry_mode,
                    id,
                    copy: false,
                    ..
                } => Some(FileChange {
                    path: location.to_vec(),
                    source: Some(source_location.to_vec()),
                    id: entry_mode.is_blob().then(|| id.detach()),
                }),
                _ => None,
            };
            if let Some(item) = item {
                result.push(item);
            }
            Ok(std::ops::ControlFlow::Continue(()))
        })
        .map_err(git_error)?;
    result.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(result)
}

/// Shared scored blob and provenance, copied only when a file changes.
struct TrackedFile {
    id: gix::ObjectId,
    state: State,
}
/// A tree map shares unchanged file contents across open branches.
type Files = BTreeMap<String, Arc<TrackedFile>>;

/// Other-parent lookup with rename sources reported by the isolated diff engine.
struct Parent<'a> {
    files: &'a Files,
    changes: Vec<FileChange>,
    renames: BTreeMap<Vec<u8>, Vec<u8>>,
}
impl Parent<'_> {
    /// Resolve the merge destination in this parent's original tree.
    fn file(&self, path: &str) -> Option<&Arc<TrackedFile>> {
        self.files.get(path).or_else(|| {
            self.renames
                .get(path.as_bytes())
                .and_then(|source| std::str::from_utf8(source).ok())
                .and_then(|source| self.files.get(source))
        })
    }
}

/// State bounded by live content plus transient changes for one commit.
#[derive(Default)]
struct Replay {
    files: Files,
    ledger: Ledger,
    warnings: BTreeSet<String>,
    formatting: u64,
}
impl Replay {
    /// Stage changes, reconcile moves across all files, then accumulate labor once.
    fn commit(
        &mut self,
        repo: &gix::Repository,
        changes: Vec<FileChange>,
        authors: &[(credit_core::AuthorId, u32)],
        options: &Options,
        excluded: &dyn Fn(&str) -> bool,
        parents: &[Parent<'_>],
    ) -> Result<bool, Error> {
        let mut staged = Vec::new();
        let mut live = BTreeMap::new();
        let mut formatting = true;
        for change in changes {
            let path = String::from_utf8(change.path).map_err(|_| {
                Error::InvalidInput("non-UTF-8 repository path cannot be reported safely".into())
            })?;
            let source = change
                .source
                .map(String::from_utf8)
                .transpose()
                .map_err(|_| {
                    Error::InvalidInput("non-UTF-8 rename source cannot be reported safely".into())
                })?;
            let previous = self.files.remove(source.as_deref().unwrap_or(&path));
            let empty = State::default();
            let previous = previous.as_ref().map_or(&empty, |file| &file.state);
            if excluded(&path) {
                formatting &= previous.is_empty();
                continue;
            }
            let text = if let Some(id) = change.id {
                let Some(text) = self.text(repo, id, &path, options.max_file_size)? else {
                    formatting &= previous.is_empty();
                    continue;
                };
                live.insert(path.clone(), id);
                text
            } else {
                String::new()
            };
            let mut result = Change::apply(previous, &text, authors, &path, options)?;
            if let Some(id) = change.id
                && let Some(file) = parents
                    .iter()
                    .filter_map(|parent| parent.file(&path))
                    .find(|file| file.id == id)
            {
                result.adopt(&file.state);
            } else {
                result.parents(
                    parents
                        .iter()
                        .filter_map(|parent| parent.file(&path))
                        .map(|file| &file.state),
                );
            }
            formatting &= result.kind() == ChangeKind::Formatting;
            if source.is_some() {
                formatting = false;
            }
            if result.pairing_capped() {
                self.warnings
                    .insert(format!("Similarity pairing capped: {path}"));
            }
            staged.push((path, result));
        }
        let sources = self.parent_move_sources(repo, parents, authors, options, excluded)?;
        let bulk = scoring::reconcile(&mut staged, sources, options.bulk_discount);
        for (path, change) in staged {
            let (state, labor) = change.finish()?;
            self.ledger.add_labor(&labor)?;
            if let Some(&id) = live.get(&path) {
                self.files.insert(path, Arc::new(TrackedFile { id, state }));
            }
        }
        self.formatting += u64::from(formatting);
        Ok(bulk)
    }

    /// Diff other parents only to recover removed blocks, never their labor.
    fn parent_move_sources(
        &mut self,
        repo: &gix::Repository,
        parents: &[Parent<'_>],
        authors: &[(credit_core::AuthorId, u32)],
        options: &Options,
        excluded: &dyn Fn(&str) -> bool,
    ) -> Result<Vec<Vec<(String, Change)>>, Error> {
        let mut sources = Vec::new();
        for parent in parents {
            let mut changes = Vec::new();
            for change in &parent.changes {
                let path = std::str::from_utf8(&change.path).map_err(|_| {
                    Error::InvalidInput(
                        "non-UTF-8 repository path cannot be reported safely".into(),
                    )
                })?;
                let source = change.source.as_deref().unwrap_or(&change.path);
                let source = std::str::from_utf8(source).map_err(|_| {
                    Error::InvalidInput("non-UTF-8 rename source cannot be reported safely".into())
                })?;
                let Some(previous) = parent.files.get(source) else {
                    continue;
                };
                if excluded(path) {
                    continue;
                }
                let text = if let Some(id) = change.id {
                    let Some(text) = self.text(repo, id, path, options.max_file_size)? else {
                        continue;
                    };
                    text
                } else {
                    String::new()
                };
                let result = Change::apply(&previous.state, &text, authors, path, options)?;
                changes.push((path.to_owned(), result));
            }
            sources.push(changes);
        }
        Ok(sources)
    }

    /// Check object size before decoding; invalid UTF-8 is scored lossily and warned.
    fn text(
        &mut self,
        repo: &gix::Repository,
        id: gix::ObjectId,
        path: &str,
        cap: u64,
    ) -> Result<Option<String>, Error> {
        let header = repo.find_header(id).map_err(git_error)?;
        if header.size() > cap {
            self.warnings
                .insert(format!("Skipped oversized file: {path}"));
            return Ok(None);
        }
        let blob = repo.find_blob(id).map_err(git_error)?;
        if blob.data.iter().take(8000).any(|&byte| byte == 0) {
            self.warnings.insert(format!("Skipped binary file: {path}"));
            return Ok(None);
        }
        let text = String::from_utf8_lossy(&blob.data);
        if matches!(text, std::borrow::Cow::Owned(_)) {
            self.warnings
                .insert(format!("Decoded invalid UTF-8 lossily: {path}"));
        }
        Ok(Some(text.into_owned()))
    }
}

/// Aggregate live ownership by the requested number of parent path components.
fn directories(files: &Files, depth: usize) -> Result<Vec<Directory>, Error> {
    if depth == 0 {
        return Ok(Vec::new());
    }
    let mut groups: BTreeMap<String, Vec<&State>> = BTreeMap::new();
    for (path, file) in files {
        let parent = path.rsplit_once('/').map_or(".", |(parent, _)| parent);
        let directory = parent.split('/').take(depth).collect::<Vec<_>>().join("/");
        groups.entry(directory).or_default().push(&file.state);
    }
    groups
        .into_iter()
        .map(|(path, files)| {
            let ownership = scoring::ownership(files.into_iter())?;
            let total = ownership
                .values()
                .try_fold(0_u64, |sum, value| sum.checked_add(*value))
                .ok_or(credit_core::Error::Overflow)?;
            Ok(Directory {
                path,
                total,
                authors: ownership,
            })
        })
        .collect()
}
