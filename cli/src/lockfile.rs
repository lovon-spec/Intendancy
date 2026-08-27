//! The exact-install lockfile (spec §9): full verified identity per entry —
//! itemID, Tree CID, registry binding, status at install, anchor block NUMBER and
//! HASH — plus per-file digests, so `intend audit` can distinguish a local
//! Registered→Absent transition from an item that was never Registered.
//! Unverified fields are absent, never placeholders. Writes are atomic.

use std::path::{Path, PathBuf};

use alloy::primitives::{Address, B256};
use eyre::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Version 2 (round-6): every entry binds the FULL deployment context id.
/// Version-1 lockfiles (pre-context records) are rejected fail-closed.
pub const LOCKFILE_VERSION: u32 = 2;

#[derive(Serialize, Deserialize, Debug, Default)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Lockfile {
    pub version: u32,
    #[serde(default)]
    pub entries: Vec<Entry>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    pub item_id: B256,
    pub tree_cid: String,
    pub chain_id: u64,
    pub registry: Address,
    /// The FULL deployment-context id (`Profile::deployment_context_id`) this
    /// entry was verified under (round-6): chain, genesis, registry, codehash,
    /// policy pins, test mode — state from one context is never consumed under
    /// another.
    pub deployment_context_id: B256,
    pub install_dir: String,
    pub installed_at_unix: u64,
    /// Contract status at install time (must have been 1 = Registered).
    pub status_at_install: u8,
    pub anchor_block: u64,
    pub anchor_block_hash: B256,
    pub anchor_state_root: B256,
    /// sha2-256 of the root block (== the Tree CID's multihash digest).
    pub root_block_sha256: String,
    pub total_bytes: u64,
    pub blocks: u64,
    pub files: Vec<LockedFile>,
    /// Latest audit result; absent until the first `intend audit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit: Option<AuditRecord>,
    /// STICKY suspension (spec §8): set when an audit observes quarantine or
    /// revocation; a later return to Registered does NOT clear it — only the
    /// explicit `intend enable` transition does, after a fresh check and a
    /// local integrity pass.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sticky_suspension: Option<String>,
    /// Install-transaction journal flag: written (and persisted) BEFORE content
    /// publishes, cleared by the finalizing save. A pending entry after a crash
    /// means the content may exist without a finalized record — audit reports
    /// it fail-closed as "incomplete"; `intend enable` finalizes it after a
    /// fresh Registered proof + integrity pass.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pending: bool,
    /// Every directory path in the verified plan (so audits detect added or
    /// removed EMPTY directories too, not only file divergence).
    #[serde(default)]
    pub dirs: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct LockedFile {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AuditRecord {
    pub checked_at_unix: u64,
    pub anchor_block: u64,
    pub anchor_block_hash: B256,
    /// Freshly proven contract status.
    pub status: u8,
    /// current | quarantined | revoked | blocked
    pub state: String,
}

impl Lockfile {
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read(path) {
            Ok(raw) => {
                let parsed: Lockfile = serde_json::from_slice(&raw)
                    .wrap_err_with(|| format!("parsing {}", path.display()))?;
                if parsed.version != LOCKFILE_VERSION {
                    bail!(
                        "lockfile {} has version {} — this build writes version \
                         {LOCKFILE_VERSION} (entries bind the full deployment context). \
                         Refusing to reuse pre-context records; migrate or reinstall \
                         deliberately",
                        path.display(),
                        parsed.version
                    );
                }
                Ok(parsed)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Lockfile {
                version: LOCKFILE_VERSION,
                entries: Vec::new(),
            }),
            Err(e) => Err(e).wrap_err_with(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        crate::store::atomic_write(path, &serde_json::to_vec_pretty(self)?)
    }

    /// Insert or replace the entry for an install directory. ONLY for
    /// finalizing an install transaction this process journaled itself (under
    /// the lockfile lock): the entry it replaces is that transaction's own
    /// pending journal. New installs must use `insert_new`.
    pub fn upsert(&mut self, entry: Entry) {
        self.entries.retain(|e| e.install_dir != entry.install_dir);
        self.entries.push(entry);
        self.entries
            .sort_by(|a, b| a.install_dir.cmp(&b.install_dir));
    }

    /// Insert a NEW entry, refusing to replace ANY existing record at the same
    /// install path (round-3 P0: journaling a fresh install must never destroy
    /// a prior entry's manifest or audit/sticky history — a repeat install
    /// fails here with the record untouched).
    pub fn insert_new(&mut self, entry: Entry) -> Result<()> {
        if let Some(prior) = self
            .entries
            .iter()
            .find(|e| e.install_dir == entry.install_dir)
        {
            if prior.pending {
                bail!(
                    "lockfile already holds a PENDING journal for {} (an earlier install \
                     did not finalize). `intend audit` reports it; if its directory exists, \
                     `intend enable` finalizes it after fresh verification; if the directory \
                     is missing, the install never published — remove the stale entry from \
                     the lockfile deliberately before reinstalling",
                    entry.install_dir
                );
            }
            bail!(
                "lockfile already records an install at {} — refusing to overwrite its \
                 manifest and audit/sticky history. Audit it, or remove the directory AND \
                 its lockfile entry deliberately before reinstalling",
                entry.install_dir
            );
        }
        self.entries.push(entry);
        self.entries
            .sort_by(|a, b| a.install_dir.cmp(&b.install_dir));
        Ok(())
    }
}

pub fn default_lockfile_path() -> PathBuf {
    PathBuf::from("intend-lock.json")
}

/// Resolve ONE stable identity for the lockfile BEFORE the data path and its
/// companion `.lock` path are derived (round-5 P1): without this, a symlink
/// alias of the lockfile takes a DIFFERENT flock than the real path (the lock
/// path is the data path + ".lock"), and an atomic save through the alias
/// would rename over the symlink itself instead of updating its target —
/// concurrent callers could split the lock and lose updates.
///
/// Rules: an EXISTING path is fully canonicalized (a symlink chain to a real
/// file resolves to its target, so every alias converges on one lock and
/// saves update the real file; a dangling symlink is rejected; a directory is
/// rejected). A NEW path canonicalizes its existing parent and joins the
/// validated final component. Static alias classes closed here (rounds 6/7):
/// the resolved target must have LINK COUNT 1 — canonicalize cannot collapse
/// HARD links, and two hard-link names would still derive distinct sidecars
/// and split state on atomic save; names ending in `.lock` are RESERVED (a
/// data lockfile named `P.lock` would collide with the companion flock path
/// of a sibling lockfile `P`, and its atomic replacement would detach a held
/// lock's inode) — the reservation is checked on BOTH the REQUESTED and the
/// RESOLVED basename (a symlink named `P.lock` must not slip through by
/// resolving to `Q.json`) under the FILESYSTEM'S folding model, not
/// bytewise: NFC-normalized + lowercased, so `P.LOCK`, mixed case, and
/// APFS's Kelvin-sign alias (`.locK` with U+212A) all reject. Lockfile
/// basenames must be valid UTF-8 (exact-path policy — this also closes the
/// non-UTF-8 raw-`.lock` bypass).
pub fn resolve_lockfile_path(path: &Path) -> Result<PathBuf> {
    fn reject_reserved(p: &Path) -> Result<()> {
        let Some(name) = p.file_name() else {
            return Ok(());
        };
        let Some(name) = name.to_str() else {
            bail!(
                "lockfile name {name:?} is not valid UTF-8 — lockfile paths must be \
                 UTF-8 (exact-path policy)"
            );
        };
        use unicode_normalization::UnicodeNormalization;
        let folded: String = name.nfc().collect::<String>().to_lowercase();
        if folded.ends_with(".lock") {
            bail!(
                "lockfile name {name:?} ends in \".lock\" under filesystem folding \
                 (case/Unicode-insensitive) — that suffix is reserved for companion \
                 flock files (a data file there would collide with a sibling \
                 lockfile's lock)"
            );
        }
        Ok(())
    }
    // The REQUESTED name is checked before any resolution: a reserved-name
    // symlink alias must not pass by resolving to an innocent basename.
    reject_reserved(path)?;
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.is_dir() {
                bail!("lockfile path {} is a directory", path.display());
            }
            let resolved = path.canonicalize().wrap_err_with(|| {
                format!(
                    "lockfile path {} does not resolve (dangling symlink?)",
                    path.display()
                )
            })?;
            let resolved_meta = std::fs::metadata(&resolved)?;
            if !resolved_meta.is_file() {
                bail!(
                    "lockfile path {} resolves to a non-regular file",
                    path.display()
                );
            }
            use std::os::unix::fs::MetadataExt;
            if resolved_meta.nlink() != 1 {
                bail!(
                    "lockfile {} has link count {} — hard-link aliases cannot be \
                     collapsed to one identity and would split the lock; use a \
                     single-link lockfile",
                    resolved.display(),
                    resolved_meta.nlink()
                );
            }
            reject_reserved(&resolved)?;
            Ok(resolved)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let name = path
                .file_name()
                .ok_or_else(|| {
                    eyre::eyre!(
                        "lockfile path {} has no usable final component",
                        path.display()
                    )
                })?
                .to_owned();
            let parent = match path.parent() {
                Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
                _ => PathBuf::from("."),
            };
            let parent = parent
                .canonicalize()
                .wrap_err_with(|| format!("lockfile parent {} must exist", parent.display()))?;
            let joined = parent.join(name);
            reject_reserved(&joined)?;
            Ok(joined)
        }
        Err(e) => Err(e).wrap_err_with(|| format!("stat lockfile path {}", path.display())),
    }
}

/// The FULL local trust context a fresh proof was made under (round-6),
/// carried into state transitions: `context_id` covers EVERY profile field
/// that defines which deployment and policy context the proof binds
/// (`Profile::deployment_context_id`); `chain_id`/`registry` ride along for
/// readable diagnostics and defense-in-depth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProofContext {
    pub chain_id: u64,
    pub registry: Address,
    pub context_id: B256,
}

/// The PRODUCTION enable/finalize transition (spec §8 re-enable; round-3/4
/// journal finalization) — the one code path both `intend enable` and the
/// crash-recovery tests execute. The caller has already located the entry
/// under the lockfile lock and freshly PROVEN `fresh_status` at an
/// authenticated anchor **under the given `ProofContext`** — the proof
/// context is bound into the transition (round-5/6 P0): the very first check
/// compares the FULL deployment-context id against the entry's recorded one,
/// so a Registered proof for the same itemID under a different registry,
/// chain, genesis, codehash, policy pins, or test mode can never clear this
/// entry's pending/sticky state. Then, in order: the
/// something-to-enable guard; the manifest guard (a pending journal without a
/// manifest was not produced by this implementation's journaling — the
/// install never completed); local integrity against the recorded manifest;
/// the fresh-Registered requirement; and, when finalizing a PENDING journal,
/// destination durability (round-4 P0: the publish rename may never have been
/// fsynced into the parent, so the finalizer re-binds the destination and
/// fsyncs its parent BEFORE the flag clears). Returns the cleared sticky
/// suspension. The caller persists the entry afterward. Nothing is mutated on
/// any failure path.
pub fn enable_transition(
    entry: &mut Entry,
    proof: &ProofContext,
    fresh_status: u8,
) -> Result<Option<String>> {
    // FIRST (round-6): the FULL deployment context must match — chain,
    // genesis, registry, codehash, policy pins, test mode. A proof made under
    // any other context (same-chain-id fork, other bytecode at the same
    // address, edited policy pins, test mode) can never touch this entry.
    if entry.deployment_context_id != proof.context_id {
        bail!(
            "entry at {:?} was verified under a DIFFERENT deployment context than \
             the fresh proof (entry {}:{} ctx {}; proof {}:{} ctx {}) — a proof \
             from another deployment/policy context can never enable this entry",
            entry.install_dir,
            entry.chain_id,
            entry.registry,
            entry.deployment_context_id,
            proof.chain_id,
            proof.registry,
            proof.context_id
        );
    }
    // Defense-in-depth numeric check (the context id already covers both).
    if entry.chain_id != proof.chain_id || entry.registry != proof.registry {
        bail!(
            "entry at {:?} is bound to {}:{} but the fresh proof is for {}:{} — \
             a proof from another registry can never enable this entry",
            entry.install_dir,
            entry.chain_id,
            entry.registry,
            proof.chain_id,
            proof.registry
        );
    }
    if entry.sticky_suspension.is_none() && !entry.pending {
        bail!(
            "entry at {:?} carries no sticky suspension and is not pending — \
             nothing to enable",
            entry.install_dir
        );
    }
    if entry.pending && entry.files.is_empty() {
        bail!(
            "entry at {:?} is a pending journal without a manifest — the install \
             never completed (foreign or legacy journal); remove the directory \
             and the stale entry, then reinstall",
            entry.install_dir
        );
    }
    verify_local_integrity(entry).wrap_err("local integrity must pass before re-enable")?;
    if fresh_status != 1 {
        bail!(
            "item status is {fresh_status} — re-enable requires a fresh \
             Registered (1) proof"
        );
    }
    if entry.pending {
        crate::car::ensure_durable_destination(Path::new(&entry.install_dir)).wrap_err(
            "the published destination could not be revalidated and made durable — \
             refusing to finalize the journal",
        )?;
    }
    let cleared = entry.sticky_suspension.take();
    entry.pending = false;
    Ok(cleared)
}

/// Pure spec-§8 audit classification. Inputs: the freshly PROVEN status, the
/// local-integrity verdict, whether the entry was installed as Registered
/// (always true by construction — used explicitly for the revocation rule), and
/// any sticky suspension. Returns (state, new sticky suspension).
pub fn classify_audit(
    fresh_status: u8,
    local_intact: bool,
    installed_as_registered: bool,
    sticky: Option<&str>,
) -> (&'static str, Option<String>) {
    // Sticky suspension is computed from the PROVEN chain status FIRST,
    // independently of what the display state ends up being — a locally
    // modified tree must still ACQUIRE quarantine/revocation stickiness
    // (review finding: modified+status-3/0 previously recorded no sticky, so a
    // later bytes+status recovery skipped the explicit re-enable).
    let new_sticky: Option<String> = match fresh_status {
        3 => Some("quarantined".into()),
        0 if installed_as_registered => Some("revoked".into()),
        _ => sticky.map(str::to_owned),
    };
    if !local_intact {
        return ("modified", new_sticky);
    }
    let state = match fresh_status {
        1 => match &new_sticky {
            Some(_) => "reenable-required",
            None => "current",
        },
        3 => "quarantined",
        0 if installed_as_registered => "revoked",
        _ => "blocked",
    };
    (state, new_sticky)
}

/// The full audit state decision for one entry: an interrupted install
/// transaction (pending journal flag) is fail-closed "incomplete" regardless of
/// chain status; otherwise the §8 classification applies. Sticky suspension is
/// still ACQUIRED from the proven status even while pending (round-3): an item
/// observed quarantined/revoked during an unfinished install must demand the
/// explicit re-enable, not slip past it when the journal later finalizes.
pub fn audit_state_for(
    entry: &Entry,
    fresh_status: u8,
    local_intact: bool,
) -> (&'static str, Option<String>) {
    let (state, sticky) = classify_audit(
        fresh_status,
        local_intact,
        entry.status_at_install == 1,
        entry.sticky_suspension.as_deref(),
    );
    if entry.pending {
        return ("incomplete", sticky);
    }
    (state, sticky)
}

/// Verify the INSTALLED BYTES against the lockfile entry: the root must be a
/// REAL directory (never a symlink); traversal checks every node with lstat
/// and never follows a node OBSERVED as a symlink; names must be valid UTF-8
/// (installed names always are, so a non-decodable name is drift — never
/// lossily folded into a recorded path); only directories and regular files
/// are legal node kinds (a FIFO/socket/device fails typed instead of hanging a
/// read); file sizes are checked from METADATA before any bytes are read, and
/// hashing streams through a bounded reader; both the exact FILE set and the
/// exact DIRECTORY set (empty dirs included) must match.
///
/// SCOPE (named §9 production blocker): these checks are point-in-time, not
/// race-free — directories are subsequently opened by PATH (read_dir) and
/// files by File::open, so a local attacker concurrently swapping nodes
/// between the lstat and the open can redirect the walk or reintroduce a
/// blocking special file. Against exclusively-local, non-concurrent
/// modification the verdict is exact; under an active concurrent attacker it
/// is advisory. Fd-relative secure traversal is future work.
pub fn verify_local_integrity(entry: &Entry) -> Result<()> {
    use sha2::{Digest, Sha256};
    let root = Path::new(&entry.install_dir);
    let root_meta = std::fs::symlink_metadata(root)
        .map_err(|_| eyre::eyre!("install dir {} is missing", entry.install_dir))?;
    if root_meta.file_type().is_symlink() {
        bail!(
            "install root {} is a SYMLINK — refusing to audit through it",
            entry.install_dir
        );
    }
    if !root_meta.is_dir() {
        bail!("install root {} is not a directory", entry.install_dir);
    }
    let mut disk_files: std::collections::BTreeMap<String, PathBuf> = Default::default();
    let mut disk_dirs: std::collections::BTreeSet<String> = Default::default();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).wrap_err_with(|| format!("reading {}", dir.display()))? {
            let e = e?;
            let path = e.path();
            // symlink_metadata: never follow anything during the audit walk.
            let meta = std::fs::symlink_metadata(&path)?;
            let ft = meta.file_type();
            let rel_os = path
                .strip_prefix(root)
                .map_err(|_| eyre::eyre!("path escape"))?;
            // Installed names are always valid UTF-8 (link names are validated
            // strings), so a non-decodable on-disk name IS drift — reject it
            // rather than lossily folding it into a path that might collide
            // with a recorded one.
            let rel = rel_os
                .to_str()
                .ok_or_else(|| eyre::eyre!("non-UTF-8 name {rel_os:?} in installed tree (drift)"))?
                .to_owned();
            if ft.is_symlink() {
                bail!("unexpected symlink {rel:?} in installed tree");
            } else if ft.is_dir() {
                disk_dirs.insert(rel);
                stack.push(path);
            } else if ft.is_file() {
                disk_files.insert(rel, path);
            } else {
                bail!(
                    "unsupported node kind at {rel:?} in installed tree (only \
                     directories and regular files are legal)"
                );
            }
        }
    }
    let expected_dirs: std::collections::BTreeSet<String> = entry.dirs.iter().cloned().collect();
    if let Some(extra) = disk_dirs.difference(&expected_dirs).next() {
        bail!("unexpected extra directory {extra:?} in installed tree");
    }
    if let Some(missing) = expected_dirs.difference(&disk_dirs).next() {
        bail!("installed directory {missing:?} is missing");
    }
    for f in &entry.files {
        let Some(path) = disk_files.remove(&f.path) else {
            bail!("installed file {:?} is missing", f.path);
        };
        // Size from metadata FIRST: an oversized replacement is rejected
        // without materializing it.
        let meta = std::fs::symlink_metadata(&path)?;
        if meta.len() != f.bytes {
            bail!(
                "installed file {:?} is {} bytes, lockfile records {}",
                f.path,
                meta.len(),
                f.bytes
            );
        }
        // Bounded streaming hash: even if the file grows between the stat and
        // the read (race), at most expected+1 bytes are consumed.
        use std::io::Read;
        let file = std::fs::File::open(&path)?;
        let mut hasher = Sha256::new();
        let mut limited = file.take(f.bytes + 1);
        let mut chunk = [0u8; 64 * 1024];
        let mut total: u64 = 0;
        loop {
            let n = limited.read(&mut chunk)?;
            if n == 0 {
                break;
            }
            total += n as u64;
            hasher.update(&chunk[..n]);
        }
        if total != f.bytes {
            bail!("installed file {:?} changed size while hashing", f.path);
        }
        let digest: [u8; 32] = hasher.finalize().into();
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        if hex != f.sha256 {
            bail!("installed file {:?} digest mismatch", f.path);
        }
    }
    if let Some((extra, _)) = disk_files.into_iter().next() {
        bail!("unexpected extra file {extra:?} in installed tree");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::classify_audit;

    #[test]
    fn classification_matrix() {
        assert_eq!(classify_audit(1, true, true, None), ("current", None));
        assert_eq!(
            classify_audit(3, true, true, None),
            ("quarantined", Some("quarantined".into()))
        );
        assert_eq!(
            classify_audit(0, true, true, None),
            ("revoked", Some("revoked".into()))
        );
        assert_eq!(classify_audit(2, true, true, None), ("blocked", None));
        // Sticky survives a return to Registered.
        assert_eq!(
            classify_audit(1, true, true, Some("revoked")),
            ("reenable-required", Some("revoked".into()))
        );
        // Local divergence dominates the DISPLAY state...
        assert_eq!(
            classify_audit(1, false, true, Some("quarantined")),
            ("modified", Some("quarantined".into()))
        );
        assert_eq!(classify_audit(1, false, true, None), ("modified", None));
        // ...but sticky is still ACQUIRED from the proven status even while
        // modified (review finding): recovery of bytes+status must NOT skip
        // the explicit re-enable.
        assert_eq!(
            classify_audit(3, false, true, None),
            ("modified", Some("quarantined".into()))
        );
        assert_eq!(
            classify_audit(0, false, true, None),
            ("modified", Some("revoked".into()))
        );
    }
}
