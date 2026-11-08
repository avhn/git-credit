//! Merge-aware object replay and live-tree aggregation.

use crate::{Directory, Error, Options, Report, git_error, identities::Authors, normalization};
use credit_core::{
    ChangeKind, FileState, Ledger, adopt_parent, apply_change, reconcile_merge_moves,
    reconcile_parents,
};
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
    let mut state = Replay {
        files: BTreeMap::new(),
        ledger: Ledger::default(),
        warnings: BTreeSet::new(),
        formatting: 0,
    };
    let mut cache = raw_diff_cache(repo.object_hash(), options.max_file_size);
    let mut retained: BTreeMap<gix::ObjectId, Files> = BTreeMap::new();
    let mut peak_retained_states = 0;
    for id in &history.order {
        let commit = repo.find_commit(*id).map_err(git_error)?;
        let shares = authors.commit(&commit, options)?;
        let tree = commit.tree().map_err(git_error)?;
        let parents = history
            .parents
            .get(id)
            .ok_or_else(|| Error::InvalidInput("missing scheduled commit".into()))?;
        let previous = if let Some(parent) = parents.first() {
            state.files = retained
                .get(parent)
                .ok_or_else(|| Error::InvalidInput("missing parent state".into()))?
                .clone();
            repo.find_commit(*parent)
                .map_err(git_error)?
                .tree()
                .map_err(git_error)?
        } else {
            state.files = BTreeMap::new();
            repo.empty_tree()
        };
        let changes = tree_changes(&previous, &tree, &mut cache)?;
        cache.clear_resource_cache();
        let other_parents = merge_parents(&repo, parents, &retained, &tree, &mut cache)?;
        state.commit(
            &repo,
            changes,
            shares.authors(),
            options,
            excluded,
            &other_parents,
        )?;
        if options.walk == crate::Walk::FirstParent && commit.parent_ids().count() > 1 {
            state.warnings.insert("Merge attribution is approximate: first-parent merge diffs credit the merge author and co-authors.".into());
        }
        // Include the current output state as well as parents still being consumed.
        peak_retained_states = peak_retained_states.max(retained.len() + 1);
        for parent in parents {
            if let Some(count) = history.remaining_children.get_mut(parent) {
                *count -= 1;
                if *count == 0 {
                    retained.remove(parent);
                }
            }
        }
        retained.insert(*id, std::mem::take(&mut state.files));
    }
    state.files = retained
        .remove(&head.id)
        .ok_or_else(|| Error::InvalidInput("missing HEAD state".into()))?;
    state
        .ledger
        .recount(state.files.values().map(|file| &file.state))?;
    let mut directories = directories(&state.files, options.depth)?;
    let authors = authors.finish(options, &mut state.ledger, &mut directories)?;
    Ok(Report {
        walk: options.walk,
        peak_retained_states,
        head: head.id.to_string(),
        commits_analyzed: u64::try_from(history.order.len())
            .map_err(|_| Error::InvalidInput("too many commits".into()))?,
        formatting_commits: state.formatting,
        authors,
        ledger: state.ledger,
        directories,
        warnings: state.warnings.into_iter().collect(),
    })
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
    state: FileState,
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
    ) -> Result<(), Error> {
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
            let empty = FileState::default();
            let previous = previous.as_ref().map_or(&empty, |file| &file.state);
            if excluded(&path) {
                formatting &= previous
                    .lines
                    .iter()
                    .all(|line| line.text.trim().is_empty());
                continue;
            }
            let text = if let Some(id) = change.id {
                let Some(text) = self.text(repo, id, &path, options.max_file_size)? else {
                    formatting &= previous
                        .lines
                        .iter()
                        .all(|line| line.text.trim().is_empty());
                    continue;
                };
                live.insert(path.clone(), id);
                text
            } else {
                String::new()
            };
            let mut result = apply_change(
                previous,
                &text,
                authors,
                normalization(&path),
                options.policy,
            )?;
            if let Some(id) = change.id
                && let Some(file) = parents
                    .iter()
                    .filter_map(|parent| parent.file(&path))
                    .find(|file| file.id == id)
            {
                adopt_parent(&mut result, &file.state);
            } else {
                reconcile_parents(
                    &mut result,
                    parents
                        .iter()
                        .filter_map(|parent| parent.file(&path))
                        .map(|file| &file.state),
                );
            }
            formatting &= result.kind == ChangeKind::Formatting;
            if source.is_some() {
                formatting = false;
            }
            if result.pairing_capped {
                self.warnings
                    .insert(format!("Similarity pairing capped: {path}"));
            }
            staged.push((path, result));
        }
        let sources = self.parent_move_sources(repo, parents, authors, options, excluded)?;
        reconcile_merge_moves(&mut staged, &sources);
        for (path, change) in staged {
            self.ledger.add_labor(&change.labor)?;
            if let Some(&id) = live.get(&path) {
                self.files.insert(
                    path,
                    Arc::new(TrackedFile {
                        id,
                        state: change.state,
                    }),
                );
            }
        }
        self.formatting += u64::from(formatting);
        Ok(())
    }

    /// Diff other parents only to recover removed blocks, never their labor.
    fn parent_move_sources(
        &mut self,
        repo: &gix::Repository,
        parents: &[Parent<'_>],
        authors: &[(credit_core::AuthorId, u32)],
        options: &Options,
        excluded: &dyn Fn(&str) -> bool,
    ) -> Result<Vec<Vec<(String, credit_core::Change)>>, Error> {
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
                let result = apply_change(
                    &previous.state,
                    &text,
                    authors,
                    normalization(path),
                    options.policy,
                )?;
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
    let mut groups: BTreeMap<String, Vec<&FileState>> = BTreeMap::new();
    for (path, file) in files {
        let parent = path.rsplit_once('/').map_or(".", |(parent, _)| parent);
        let directory = parent.split('/').take(depth).collect::<Vec<_>>().join("/");
        groups.entry(directory).or_default().push(&file.state);
    }
    groups
        .into_iter()
        .map(|(path, files)| {
            let mut ledger = Ledger::default();
            ledger.recount(files)?;
            let total = ledger
                .ownership
                .values()
                .try_fold(0_u64, |sum, value| sum.checked_add(*value))
                .ok_or(credit_core::Error::Overflow)?;
            Ok(Directory {
                path,
                total,
                authors: ledger.ownership,
            })
        })
        .collect()
}
