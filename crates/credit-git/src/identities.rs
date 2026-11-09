//! HEAD mailmap resolution, identity grouping and privacy-safe final aggregation.

use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
};

use credit_core::{AuthorId, LaborDelta, Ledger, LineOwner};
use gix::bstr::ByteSlice;

use crate::{Author, Directory, Error, GroupBy, Options, git_error};

/// A resolved group and per-email commit participation counts.
struct Group {
    name: String,
    emails: BTreeMap<String, u64>,
}
/// Caller-owned deterministic interner. Privacy is applied after the complete walk
/// so discovering an opted-out alias later also anonymizes the group's earlier work.
pub(crate) struct Authors {
    groups: Vec<Group>,
    ids: BTreeMap<String, AuthorId>,
    map: gix::mailmap::Snapshot,
}
impl Authors {
    /// Read the committed map, then optional configured blob and file overrides.
    pub(crate) fn load(repo: &gix::Repository, head: &gix::Commit<'_>) -> Result<Self, Error> {
        let mut map = gix::mailmap::Snapshot::default();
        if let Some(entry) = head.tree().map_err(git_error)?.find_entry(".mailmap") {
            let blob = entry
                .object()
                .map_err(git_error)?
                .try_into_blob()
                .map_err(git_error)?;
            map.merge(gix::mailmap::parse_ignore_errors(&blob.data));
        }
        let config = repo.config_snapshot();
        if let Some(spec) = config.string("mailmap.blob") {
            let blob = repo
                .rev_parse_single(spec.as_bstr())
                .map_err(git_error)?
                .object()
                .map_err(git_error)?
                .try_into_blob()
                .map_err(git_error)?;
            map.merge(gix::mailmap::parse_ignore_errors(&blob.data));
        }
        if let Some(path) = config.trusted_path("mailmap.file").map_err(git_error)? {
            let root = repo.workdir().unwrap_or_else(|| repo.git_dir());
            let mut bytes = Vec::new();
            std::fs::File::open(root.join(path))?
                .take(1_048_577)
                .read_to_end(&mut bytes)?;
            if bytes.len() > 1_048_576 {
                return Err(Error::InvalidInput("mailmap.file exceeds 1 MiB".into()));
            }
            map.merge(gix::mailmap::parse_ignore_errors(&bytes));
        }
        Ok(Self {
            groups: Vec::new(),
            ids: BTreeMap::new(),
            map,
        })
    }

    /// Stable grouping keys let cached numeric IDs follow the current cold walk.
    pub(crate) fn keys(&self) -> BTreeMap<u32, String> {
        self.ids
            .iter()
            .map(|(key, id)| (id.0, key.clone()))
            .collect()
    }

    /// Resolve participants and split once per distinct group. Count each email
    /// once per commit, even if repeated in trailers, for representative selection.
    pub(crate) fn commit(
        &mut self,
        commit: &gix::Commit<'_>,
        options: &Options,
    ) -> Result<LineOwner, Error> {
        let decoded = commit.decode().map_err(git_error)?;
        let author = decoded.author().map_err(git_error)?;
        let mut people = vec![self.resolve(author)];
        let message = String::from_utf8_lossy(decoded.message);
        let tail = message
            .trim_end()
            .lines()
            .rev()
            .take_while(|line| !line.trim().is_empty());
        for line in tail {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            if !key.trim().eq_ignore_ascii_case("co-authored-by") {
                continue;
            }
            let Some((name, email)) = value.trim().rsplit_once('<') else {
                continue;
            };
            let Some(email) = email.strip_suffix('>') else {
                continue;
            };
            if email.trim().is_empty() || name.trim().is_empty() {
                continue;
            }
            let signature = gix::actor::SignatureRef {
                name: name.trim().as_bytes().as_bstr(),
                email: email.trim().as_bytes().as_bstr(),
                time: author.time,
            };
            people.push(self.resolve(signature));
        }
        let mut seen = BTreeSet::new();
        let mut groups = BTreeMap::new();
        for (name, email) in people {
            let key = group_key(&name, &email, options.group_by);
            if !seen.insert((key.clone(), email.clone())) {
                continue;
            }
            let id = self.intern(&key, name, email)?;
            groups.insert(key, id);
        }
        LineOwner::equal(&groups.into_values().collect::<Vec<_>>()).map_err(Into::into)
    }

    /// Resolve canonical spelling first, without prematurely applying opt-out.
    fn resolve(&self, signature: gix::actor::SignatureRef<'_>) -> (String, String) {
        let resolved = self.map.resolve(signature);
        (
            resolved.name.to_str_lossy().trim().to_owned(),
            resolved.email.to_str_lossy().trim().to_lowercase(),
        )
    }

    /// Allocate stable replay IDs and collect frequencies independently of credit.
    fn intern(&mut self, key: &str, name: String, email: String) -> Result<AuthorId, Error> {
        let id = if let Some(id) = self.ids.get(key) {
            *id
        } else {
            let id = author_id(self.groups.len())?;
            self.ids.insert(key.to_owned(), id);
            self.groups.push(Group {
                name,
                emails: BTreeMap::new(),
            });
            id
        };
        let group = self
            .groups
            .get_mut(id.0 as usize)
            .ok_or_else(|| Error::InvalidInput("invalid identity index".into()))?;
        let count = group.emails.entry(email).or_default();
        *count = count.checked_add(1).ok_or(credit_core::Error::Overflow)?;
        Ok(id)
    }

    /// Choose representative addresses and collapse opted-out groups, remapping
    /// every ledger and directory atomically before the report becomes visible.
    pub(crate) fn finish(
        self,
        options: &Options,
        ledger: &mut Ledger,
        directories: &mut [Directory],
        timeline: Option<&mut crate::Timeline>,
    ) -> Result<Vec<Author>, Error> {
        let mut authors = Vec::new();
        let mut mapping = BTreeMap::new();
        let mut anonymous = None;
        for (old, group) in self.groups.into_iter().enumerate() {
            let opted_out = group.emails.keys().any(|email| {
                options
                    .opt_out
                    .iter()
                    .any(|item| item.trim().eq_ignore_ascii_case(email))
            });
            let id = if opted_out {
                if let Some(id) = anonymous {
                    id
                } else {
                    let id = author_id(authors.len())?;
                    authors.push(Author {
                        name: "anonymous".into(),
                        email: String::new(),
                        emails: Vec::new(),
                    });
                    anonymous = Some(id);
                    id
                }
            } else {
                let email = group
                    .emails
                    .iter()
                    .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
                    .map(|(email, _)| email.clone())
                    .unwrap_or_default();
                let id = author_id(authors.len())?;
                authors.push(Author {
                    name: group.name,
                    email,
                    emails: group.emails.into_keys().collect(),
                });
                id
            };
            mapping.insert(author_id(old)?, id);
        }
        let ownership = remap(&ledger.ownership, &mapping)?;
        let labor = remap(&ledger.labor, &mapping)?;
        let remapped_directories = directories
            .iter()
            .map(|directory| remap(&directory.authors, &mapping))
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(timeline) = timeline {
            for bucket in &mut timeline.activity {
                bucket.authors = remap(&bucket.authors, &mapping)?;
            }
            for sample in &mut timeline.ownership {
                sample.authors = remap(&sample.authors, &mapping)?;
            }
        }
        ledger.ownership = ownership;
        ledger.labor = labor;
        for (directory, authors) in directories.iter_mut().zip(remapped_directories) {
            directory.authors = authors;
        }
        Ok(authors)
    }
}

/// Empty names cannot safely identify a person, so fall back to canonical email.
fn group_key(name: &str, email: &str, grouping: GroupBy) -> String {
    match grouping {
        GroupBy::Name if !name.trim().is_empty() => format!("name:{}", name.trim().to_lowercase()),
        _ => format!("email:{email}"),
    }
}

/// Validate the interner index at the allocation boundary.
fn author_id(index: usize) -> Result<AuthorId, Error> {
    u32::try_from(index)
        .map(AuthorId)
        .map_err(|_| Error::InvalidInput("too many authors".into()))
}

/// Remap fixed-point totals without changing their sum or silently overflowing.
fn remap(totals: &LaborDelta, mapping: &BTreeMap<AuthorId, AuthorId>) -> Result<LaborDelta, Error> {
    let mut result = BTreeMap::new();
    for (old, amount) in totals {
        let id = mapping
            .get(old)
            .ok_or_else(|| Error::InvalidInput("missing identity mapping".into()))?;
        let total = result.entry(*id).or_insert(0_u64);
        *total = total
            .checked_add(*amount)
            .ok_or(credit_core::Error::Overflow)?;
    }
    Ok(result)
}
