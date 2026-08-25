//! Local verified state: the catalog (last fully verified snapshot + meta) and
//! the per-registry finalized-anchor high-water mark (spec §8 rollback
//! resistance). All writes are atomic (temp file + fsync + rename — the Gate 1
//! pattern), and the high-water file is versioned with unknown-field rejection.

use std::path::{Path, PathBuf};

use alloy::primitives::{Address, B256};
use eyre::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::anchor::QuorumAnchor;
use crate::snapshot::Snapshot;

pub fn default_state_dir() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("INTEND_STATE_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var("HOME").wrap_err("HOME is not set (use --state-dir)")?;
    Ok(PathBuf::from(home).join(".intend"))
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".tmp-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    ));
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .wrap_err_with(|| format!("creating {}", tmp.display()))?;
    let result = f
        .write_all(bytes)
        .and_then(|()| f.sync_all())
        .map_err(eyre::Report::from)
        .and_then(|()| {
            drop(f);
            crate::failpoint("atomic-write-pre-rename");
            std::fs::rename(&tmp, path).wrap_err_with(|| format!("publishing {}", path.display()))
        })
        .and_then(|()| {
            // Durability of the RENAME itself requires fsyncing the parent
            // directory (review finding: atomic_write lacked parent fsync).
            std::fs::File::open(dir)
                .and_then(|d| d.sync_all())
                .wrap_err_with(|| format!("fsync of parent dir {}", dir.display()))
        });
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Advisory interprocess lock (exclusive `flock`) serializing read-modify-write
/// of a state scope — the high-water/catalog state dir, or a lockfile. Held for
/// the lifetime of the guard; concurrent `intend` processes block rather than
/// racing high-water regressions or losing lockfile entries.
#[derive(Debug)]
pub struct ScopeLock {
    _file: std::fs::File,
}

/// Open a lock SIDECAR safely (round-7): the final component is opened with
/// O_NOFOLLOW — a pre-existing SYMLINK at the sidecar path is an alias attack
/// (`P.lock -> Q.json` would flock the CURRENT Q data inode, which a Q-side
/// atomic save then detaches, splitting the serialization) — and O_NONBLOCK,
/// so a pre-existing FIFO cannot hang the open before the type check. The
/// opened fd is then fstat'ed and must be a REGULAR, EMPTY, single-link file:
/// sidecars are dedicated — created empty and never written — so anything
/// else at that path is not ours and is refused, never locked through.
fn open_sidecar(lock_path: &Path) -> Result<std::fs::File> {
    use std::os::fd::FromRawFd;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    if let Some(dir) = lock_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let c = std::ffi::CString::new(lock_path.as_os_str().as_bytes())
        .wrap_err("lock path contains NUL")?;
    let fd = unsafe {
        libc::open(
            c.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            0o644 as libc::c_uint,
        )
    };
    if fd < 0 {
        let e = std::io::Error::last_os_error();
        let hint = if e.raw_os_error() == Some(libc::ELOOP) {
            " (a SYMLINK sits at the sidecar path — refusing to follow it)"
        } else {
            ""
        };
        return Err(e)
            .wrap_err_with(|| format!("opening lock sidecar {}{hint}", lock_path.display()));
    }
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    let meta = file.metadata().wrap_err("fstat lock sidecar")?;
    if !meta.is_file() || meta.len() != 0 || meta.nlink() != 1 {
        bail!(
            "lock sidecar {} is not a dedicated empty regular file (regular: {}, \
             {} bytes, {} links) — something else occupies the lock path; refusing \
             to lock through it",
            lock_path.display(),
            meta.is_file(),
            meta.len(),
            meta.nlink()
        );
    }
    Ok(file)
}

impl ScopeLock {
    pub fn acquire(lock_path: &Path) -> Result<Self> {
        use std::os::unix::io::AsRawFd;
        let file = open_sidecar(lock_path)?;
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        if rc != 0 {
            return Err(std::io::Error::last_os_error())
                .wrap_err_with(|| format!("flock {}", lock_path.display()));
        }
        Ok(Self { _file: file })
    }

    /// Nonblocking variant (used by tests to prove exclusivity).
    pub fn try_acquire(lock_path: &Path) -> Result<Option<Self>> {
        use std::os::unix::io::AsRawFd;
        let file = open_sidecar(lock_path)?;
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc != 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EWOULDBLOCK) {
                return Ok(None);
            }
            return Err(err).wrap_err_with(|| format!("flock {}", lock_path.display()));
        }
        Ok(Some(Self { _file: file }))
    }
}

pub fn state_lock_path(state_dir: &Path) -> std::path::PathBuf {
    state_dir.join(".lock")
}

pub fn lockfile_lock_path(lockfile: &Path) -> std::path::PathBuf {
    let mut os = lockfile.as_os_str().to_owned();
    os.push(".lock");
    std::path::PathBuf::from(os)
}

// ---------- high-water mark ----------

#[derive(Serialize, Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct HighWaterFile {
    pub version: u32,
    #[serde(default)]
    pub marks: std::collections::BTreeMap<String, Mark>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Mark {
    pub block: u64,
    pub hash: B256,
}

#[derive(Debug)]
pub struct HighWater {
    path: PathBuf,
    file: HighWaterFile,
}

impl HighWater {
    /// Keys are genesis-qualified (round-6): a same-chain-id fork with another
    /// genesis keeps INDEPENDENT rollback marks. Deliberately NOT the full
    /// deployment-context id: rollback protection guards the chain view and
    /// must survive local policy-pin or test-mode edits to the profile.
    pub fn key(chain_id: u64, genesis_hash: B256, registry: Address) -> String {
        format!("{chain_id}:{genesis_hash:#x}:{registry:#x}")
    }

    pub fn load(state_dir: &Path) -> Result<Self> {
        let path = state_dir.join("highwater.json");
        let file = match std::fs::read(&path) {
            Ok(raw) => {
                let parsed: HighWaterFile = serde_json::from_slice(&raw)
                    .wrap_err_with(|| format!("parsing {}", path.display()))?;
                if parsed.version != 2 {
                    bail!(
                        "high-water file {} has version {} — this build writes version 2 \
                         (genesis-qualified keys). Pre-context marks are ambiguous; after \
                         confirming no rollback concern, delete the file and re-run \
                         `intend update`",
                        path.display(),
                        parsed.version
                    );
                }
                parsed
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HighWaterFile {
                version: 2,
                marks: Default::default(),
            },
            Err(e) => return Err(e).wrap_err_with(|| format!("reading {}", path.display())),
        };
        Ok(Self { path, file })
    }

    /// Enforce monotonic finalized anchors (spec §8): an older block, or a
    /// conflicting hash at the recorded height, rejects; a newer block advances
    /// the mark durably. Returns whether the mark advanced.
    pub fn observe(
        &mut self,
        chain_id: u64,
        genesis_hash: B256,
        registry: Address,
        anchor: &QuorumAnchor,
    ) -> Result<bool> {
        let key = Self::key(chain_id, genesis_hash, registry);
        if let Some(mark) = self.file.marks.get(&key) {
            if anchor.block_number < mark.block {
                bail!(
                    "anchor rollback refused: block {} is below the recorded high-water \
                     mark {} for {key} (explicit recovery required)",
                    anchor.block_number,
                    mark.block
                );
            }
            if anchor.block_number == mark.block && anchor.block_hash != mark.hash {
                bail!(
                    "anchor conflict refused: block {} hash {} contradicts the recorded \
                     hash {} for {key}",
                    anchor.block_number,
                    anchor.block_hash,
                    mark.hash
                );
            }
            if anchor.block_number == mark.block {
                return Ok(false);
            }
        }
        self.file.marks.insert(
            key,
            Mark {
                block: anchor.block_number,
                hash: anchor.block_hash,
            },
        );
        atomic_write(&self.path, &serde_json::to_vec_pretty(&self.file)?)?;
        Ok(true)
    }
}

// ---------- verified catalog ----------

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CatalogMeta {
    /// The FULL deployment-context id (`Profile::deployment_context_id`) this
    /// catalog was verified under (round-6). Older metas without the field
    /// fail to parse — fail closed; re-run `intend update`.
    pub deployment_context_id: B256,
    pub verified_at_unix: u64,
    pub anchor_block: u64,
    pub anchor_block_hash: B256,
    pub anchor_state_root: B256,
    pub anchor_mode: String,
    pub items: u64,
    pub snapshot_sha256: String,
}

#[derive(Debug)]
pub struct Catalog {
    pub meta: CatalogMeta,
    pub snapshot: Snapshot,
}

impl Catalog {
    /// The catalog is only meaningful under the profile it was verified for
    /// (round-5/6): a state dir last updated for registry A — or under a
    /// different genesis, codehash, policy pins, or test mode of the SAME
    /// numeric binding — must not be displayed or used for name/completeness
    /// resolution while another profile is active. Callers check this
    /// immediately after `load`.
    pub fn assert_bound_to(
        &self,
        chain_id: u64,
        registry: Address,
        context_id: B256,
    ) -> Result<()> {
        let b = &self.snapshot.binding;
        if b.chain_id != chain_id || b.registry != registry {
            bail!(
                "the local catalog was verified for {}:{} but the active profile pins \
                 {}:{} — run `intend update` under this profile (or point --state-dir \
                 at the right state)",
                b.chain_id,
                b.registry,
                chain_id,
                registry
            );
        }
        if self.meta.deployment_context_id != context_id {
            bail!(
                "the local catalog was verified under a DIFFERENT deployment context \
                 (same {}:{}, but the genesis/codehash/policy-pin/test-mode context \
                 differs: catalog {} vs profile {}) — run `intend update` under this \
                 profile",
                chain_id,
                registry,
                self.meta.deployment_context_id,
                context_id
            );
        }
        Ok(())
    }

    fn paths(state_dir: &Path) -> (PathBuf, PathBuf) {
        (
            state_dir.join("catalog-meta.json"),
            state_dir.join("catalog-snapshot.json"),
        )
    }

    pub fn save(state_dir: &Path, meta: &CatalogMeta, snapshot: &Snapshot) -> Result<()> {
        let (meta_path, snap_path) = Self::paths(state_dir);
        atomic_write(&snap_path, &serde_json::to_vec(snapshot)?)?;
        atomic_write(&meta_path, &serde_json::to_vec_pretty(meta)?)?;
        Ok(())
    }

    pub fn load(state_dir: &Path) -> Result<Self> {
        let (meta_path, snap_path) = Self::paths(state_dir);
        let meta: CatalogMeta = serde_json::from_slice(
            &std::fs::read(&meta_path)
                .wrap_err("no verified catalog — run `intend update` first")?,
        )
        .wrap_err_with(|| format!("parsing {}", meta_path.display()))?;
        let raw = std::fs::read(&snap_path)
            .wrap_err("catalog snapshot missing — run `intend update` again")?;
        use sha2::{Digest, Sha256};
        let digest = crate::car::hex_lower(&Sha256::digest(&raw));
        if digest != meta.snapshot_sha256 {
            bail!(
                "catalog snapshot does not match its recorded digest (corrupt or \
                 tampered local state) — run `intend update` again"
            );
        }
        let snapshot: Snapshot = serde_json::from_slice(&raw)
            .wrap_err_with(|| format!("parsing {}", snap_path.display()))?;
        Ok(Self { meta, snapshot })
    }
}
