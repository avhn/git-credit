//! Bounded, checksummed replay packs. All serialization remains outside core.

use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::Path,
};

use credit_core::{AuthorId, LineOwner};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    Error, Options, git_error,
    scoring::{Change, State},
};

/// Maximum encoded pack size, including its checksum (256 MiB).
const MAX_BYTES: u64 = 256 * 1024 * 1024;
/// Cache representation version, independent of attribution semantics.
const FORMAT: u32 = 3;

/// Validated attribution shares encoded without exposing serde in core.
type Shares = Vec<(u32, u32)>;

/// One commit's additive work, in original integer units.
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Commit {
    pub(crate) identity: Shares,
    pub(crate) labor: BTreeMap<u32, u64>,
    pub(crate) formatting: bool,
    pub(crate) bulk: bool,
    pub(crate) warnings: Vec<String>,
}

/// Integer ownership summary, never a historical file map.
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Summary {
    pub(crate) total: u64,
    pub(crate) authors: BTreeMap<u32, u64>,
}

/// Fingerprint numeric allocation order because token ownership rounds in ID order.
pub(crate) fn allocation_key(groups: &BTreeMap<u32, String>) -> Result<String, Error> {
    Ok(digest(&postcard::to_allocvec(groups).map_err(invalid)?))
}

/// One immutable file state, addressed by the digest of its encoded bytes.
#[derive(Serialize, Deserialize)]
pub(crate) struct File {
    pub(crate) blob: String,
    pub(crate) owners: Vec<Shares>,
}

/// Complete self-contained generation; unchanged file states occur only once.
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Pack {
    pub(crate) key: String,
    pub(crate) identity_key: String,
    pub(crate) allocation_key: String,
    pub(crate) ownership: BTreeMap<String, Summary>,
    pub(crate) groups: BTreeMap<u32, String>,
    pub(crate) head: String,
    pub(crate) commits: BTreeMap<String, Commit>,
    pub(crate) snapshots: BTreeMap<String, BTreeMap<String, String>>,
    pub(crate) files: BTreeMap<String, Vec<u8>>,
}

/// Turn an encoding or validation failure into the library's typed input error.
fn invalid(message: impl std::fmt::Display) -> Error {
    Error::InvalidInput(format!("invalid replay cache: {message}"))
}

/// SHA-256 is stable across processes and platforms, unlike default Rust hashing.
fn digest(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut result, byte| {
            let _ = write!(result, "{byte:02x}");
            result
        })
}

/// Fingerprint every scoring option and the caller's complete exclusion policy.
pub(crate) fn key(options: &Options) -> Result<String, Error> {
    let exclusions = options.cache.as_ref().map(|cache| &cache.exclusions);
    let bytes = postcard::to_allocvec(&(
        FORMAT,
        crate::ALGORITHM_VERSION,
        options.granularity.as_str(),
        options.walk.as_str(),
        options.policy.keep_per_mille,
        options.bulk_discount,
        options.max_file_size,
        exclusions,
    ))
    .map_err(invalid)?;
    Ok(digest(&bytes))
}

/// Preserve order because co-author remainder allocation depends on it.
pub(crate) fn shares(owner: &LineOwner) -> Shares {
    owner
        .authors()
        .iter()
        .map(|(id, weight)| (id.0, *weight))
        .collect()
}

/// Read a bounded generation, verify its checksum before decoding any structure.
pub(crate) fn load(directory: &Path) -> Result<Pack, Error> {
    let file = std::fs::File::open(directory.join("replay.pack"))?;
    if file.metadata()?.len() > MAX_BYTES {
        return Err(invalid("pack exceeds 256 MiB"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() < 32 || u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_BYTES {
        return Err(invalid("truncated or oversized pack"));
    }
    let (checksum, payload) = bytes.split_at(32);
    if Sha256::digest(payload).as_slice() != checksum {
        return Err(invalid("checksum mismatch"));
    }
    let (pack, remaining) = postcard::take_from_bytes(payload).map_err(invalid)?;
    if !remaining.is_empty() {
        return Err(invalid("trailing bytes"));
    }
    Ok(pack)
}

/// Encode and atomically replace one generation, retaining the old pack on failure.
/// A pack beyond the fixed bound is not published. No unrelated files are removed.
pub(crate) fn save(directory: &Path, pack: &Pack) -> Result<usize, Error> {
    let bytes = postcard::to_allocvec(pack).map_err(invalid)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_BYTES - 32 {
        return Err(invalid("generation exceeds 256 MiB; cache not written"));
    }
    std::fs::create_dir_all(directory)?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary.write_all(&Sha256::digest(&bytes))?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(directory.join("replay.pack"))
        .map_err(|error| Error::Io(error.error))?;
    Ok(bytes.len() + 32)
}

/// Store blob identity and provenance, deduplicating by serialized-state hash.
pub(crate) fn store_file(
    pack: &mut Pack,
    blob: gix::ObjectId,
    state: &State,
) -> Result<String, Error> {
    let owners = match state {
        State::Empty => Vec::new(),
        State::Line(state) => state.lines.iter().map(|line| shares(&line.owner)).collect(),
        State::Token(state) => state
            .tokens()
            .iter()
            .map(|token| shares(token.owner()))
            .collect(),
    };
    let bytes = postcard::to_allocvec(&File {
        blob: blob.to_string(),
        owners,
    })
    .map_err(invalid)?;
    let hash = digest(&bytes);
    pack.files.entry(hash.clone()).or_insert(bytes);
    Ok(hash)
}

/// Re-lex immutable Git content instead of trusting cached token weights or bounds.
/// Validate every owner and author ID before allowing provenance into replay.
pub(crate) fn restore_file(
    repo: &gix::Repository,
    pack: &Pack,
    hash: &str,
    path: &str,
    options: &Options,
    author_count: usize,
) -> Result<(gix::ObjectId, State), Error> {
    let bytes = pack
        .files
        .get(hash)
        .ok_or_else(|| invalid("missing file state"))?;
    if digest(bytes) != hash {
        return Err(invalid("file checksum mismatch"));
    }
    let (file, rest): (File, _) = postcard::take_from_bytes(bytes).map_err(invalid)?;
    if !rest.is_empty() {
        return Err(invalid("trailing file bytes"));
    }
    let id = gix::ObjectId::from_hex(file.blob.as_bytes()).map_err(invalid)?;
    if repo.find_header(id).map_err(git_error)?.size() > options.max_file_size {
        return Err(invalid("oversized cached blob"));
    }
    let blob = repo.find_blob(id).map_err(git_error)?;
    if blob.data.iter().take(8000).any(|byte| *byte == 0) {
        return Err(invalid("binary cached blob"));
    }
    let text = String::from_utf8_lossy(&blob.data);
    let (mut state, _) = Change::apply(
        &State::Empty,
        &text,
        &[(AuthorId(0), credit_core::SCALE)],
        path,
        options,
    )?
    .finish()?;
    let owners = file
        .owners
        .into_iter()
        .map(|shares| {
            if shares
                .iter()
                .any(|(id, _)| usize::try_from(*id).map_or(true, |id| id >= author_count))
            {
                return Err(invalid("owner outside identity table"));
            }
            LineOwner::new(
                &shares
                    .into_iter()
                    .map(|(id, weight)| (AuthorId(id), weight))
                    .collect::<Vec<_>>(),
            )
            .map_err(Into::into)
        })
        .collect::<Result<Vec<_>, Error>>()?;
    match &mut state {
        State::Line(state) => {
            if state.lines.len() != owners.len() {
                return Err(invalid("line count mismatch"));
            }
            for (line, owner) in state.lines.iter_mut().zip(owners) {
                line.owner = owner;
            }
        }
        State::Token(state) => state.restore_owners(owners)?,
        State::Empty => return Err(invalid("unexpected empty representation")),
    }
    Ok((id, state))
}

/// Remap encounter-order IDs without changing co-author order or share rounding.
/// Group membership changes still fail subsequent per-commit fingerprint checks.
pub(crate) fn remap_identities(
    pack: &mut Pack,
    current: &BTreeMap<u32, String>,
) -> Result<(), Error> {
    let reverse: BTreeMap<_, _> = current.iter().map(|(id, key)| (key, *id)).collect();
    let mapping = pack
        .groups
        .iter()
        .map(|(id, key)| {
            reverse
                .get(key)
                .copied()
                .map(|new| (*id, new))
                .ok_or_else(|| invalid("identity digest changed (group membership)"))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let remap = |shares: &mut Shares| -> Result<(), Error> {
        for (id, _) in shares {
            *id = *mapping
                .get(id)
                .ok_or_else(|| invalid("unknown cached identity"))?;
        }
        Ok(())
    };
    for record in pack.commits.values_mut() {
        remap(&mut record.identity)?;
        record.labor = std::mem::take(&mut record.labor)
            .into_iter()
            .map(|(id, amount)| {
                mapping
                    .get(&id)
                    .copied()
                    .map(|id| (id, amount))
                    .ok_or_else(|| invalid("unknown labor identity"))
            })
            .collect::<Result<_, _>>()?;
    }
    let mut hashes = BTreeMap::new();
    let mut files = BTreeMap::new();
    for (hash, bytes) in &pack.files {
        if digest(bytes) != *hash {
            return Err(invalid("file checksum mismatch"));
        }
        let (mut file, rest): (File, _) = postcard::take_from_bytes(bytes).map_err(invalid)?;
        if !rest.is_empty() {
            return Err(invalid("trailing file bytes"));
        }
        for owner in &mut file.owners {
            remap(owner)?;
        }
        let bytes = postcard::to_allocvec(&file).map_err(invalid)?;
        let new = digest(&bytes);
        hashes.insert(hash.clone(), new.clone());
        files.insert(new, bytes);
    }
    for snapshot in pack.snapshots.values_mut() {
        for hash in snapshot.values_mut() {
            *hash = hashes
                .get(hash)
                .ok_or_else(|| invalid("missing file state"))?
                .clone();
        }
    }
    pack.files = files;
    pack.groups.clone_from(current);
    Ok(())
}

/// Hash stable group keys and ordered shares for exactly the processed commit set.
pub(crate) fn identity_key<'a>(
    processed: impl Iterator<Item = &'a String>,
    identities: &BTreeMap<String, LineOwner>,
    groups: &BTreeMap<u32, String>,
) -> Result<String, Error> {
    let mut fingerprints = Vec::new();
    for id in processed {
        let owner = identities
            .get(id)
            .ok_or_else(|| invalid("missing processed identity"))?;
        let shares = owner
            .authors()
            .iter()
            .map(|(id, share)| {
                groups
                    .get(&id.0)
                    .map(|key| (key, *share))
                    .ok_or_else(|| invalid("missing identity group"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        fingerprints.push((id, shares));
    }
    Ok(digest(
        &postcard::to_allocvec(&fingerprints).map_err(invalid)?,
    ))
}
