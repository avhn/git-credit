//! Deterministic parent-before-child scheduling and state lifetime counts.

use std::collections::{BTreeMap, BTreeSet};

use crate::{Error, Walk, git_error};

/// Reachable graph metadata, without retaining historical tree content.
pub(crate) struct History {
    pub(crate) order: Vec<gix::ObjectId>,
    pub(crate) parents: BTreeMap<gix::ObjectId, Vec<gix::ObjectId>>,
    pub(crate) remaining_children: BTreeMap<gix::ObjectId, usize>,
}

impl History {
    /// Read each reachable commit once and schedule ready commits by object ID.
    pub(crate) fn load(
        repo: &gix::Repository,
        head: gix::ObjectId,
        walk: Walk,
    ) -> Result<Self, Error> {
        let mut parents = BTreeMap::new();
        let mut pending = vec![head];
        let mut children: BTreeMap<_, BTreeSet<_>> = BTreeMap::new();
        while let Some(id) = pending.pop() {
            if parents.contains_key(&id) {
                continue;
            }
            let commit = repo.find_commit(id).map_err(git_error)?;
            let decoded = commit.decode().map_err(git_error)?;
            let mut ids = Vec::new();
            for hex in &decoded.parents {
                let parent = gix::ObjectId::from_hex(hex).map_err(git_error)?;
                if !ids.contains(&parent) {
                    ids.push(parent);
                    pending.push(parent);
                    children.entry(parent).or_default().insert(id);
                }
                if walk == Walk::FirstParent {
                    break;
                }
            }
            parents.insert(id, ids);
        }
        let mut outstanding: BTreeMap<_, _> = parents
            .iter()
            .map(|(&id, parents)| (id, parents.len()))
            .collect();
        let mut ready: BTreeSet<_> = outstanding
            .iter()
            .filter_map(|(&id, &count)| (count == 0).then_some(id))
            .collect();
        let mut order = Vec::with_capacity(parents.len());
        while let Some(id) = ready.pop_first() {
            order.push(id);
            if let Some(children) = children.get(&id) {
                for child in children {
                    if let Some(count) = outstanding.get_mut(child) {
                        *count -= 1;
                        if *count == 0 {
                            ready.insert(*child);
                        }
                    }
                }
            }
        }
        if order.len() != parents.len() {
            return Err(Error::InvalidInput("commit graph contains a cycle".into()));
        }
        let remaining_children = children
            .into_iter()
            .map(|(id, children)| (id, children.len()))
            .collect();
        Ok(Self {
            order,
            parents,
            remaining_children,
        })
    }
}
