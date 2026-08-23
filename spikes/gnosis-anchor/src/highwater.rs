//! Per-registry finalized-anchor high-water mark (RFC 0001 §2 rollback resistance).
//!
//! A provider must not be able to roll a client back to an older finalized block, or
//! substitute a different hash at the same height, without an explicit operator
//! recovery action (`--allow-rollback`). Violations are typed [`StageError`]s.
//!
//! The state file is VERSIONED and parsed strictly (`deny_unknown_fields`, required
//! fields): any file that is not exactly a recognized high-water state — a stray
//! capture `meta.json`, an empty object, a future version — is a typed error, never
//! silently treated as an empty database (review blocker 1).
//!
//! Persistence is atomic (temp file + rename) but NOT concurrency-safe across
//! processes — a documented spike limitation.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use alloy::primitives::{Address, B256};
use eyre::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::atomic_file::replace_atomically;
use crate::errors::StageError;

pub const STATE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Mark {
    pub block_number: u64,
    pub block_hash: B256,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// First anchor recorded for this registry.
    Initialized,
    /// Anchor advanced past the stored mark.
    Advanced,
    /// Same height, same hash as stored.
    Unchanged,
    /// Operator explicitly accepted an older/conflicting anchor.
    RecoveryOverride,
}

/// Strict on-disk schema: `version` and `marks` are REQUIRED and no other field is
/// accepted, so foreign JSON (e.g. `CaptureMeta`) cannot masquerade as empty state.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StateFile {
    version: u32,
    marks: BTreeMap<String, Mark>, // key: "chainId:address" (lowercase)
}

#[derive(Debug)]
pub struct HighWater {
    path: Option<PathBuf>,
    state: StateFile,
}

impl HighWater {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let state = match path {
            Some(p) => match std::fs::symlink_metadata(p) {
                Ok(metadata) => {
                    if metadata.file_type().is_symlink() {
                        return Err(StageError::InvalidHighWaterState {
                            path: p.display().to_string(),
                            detail: "state-file leaf is a symbolic link; use a regular file path"
                                .into(),
                        }
                        .into());
                    }
                    let raw = std::fs::read_to_string(p)
                        .wrap_err_with(|| format!("reading high-water state {}", p.display()))?;
                    let parsed: StateFile = serde_json::from_str(&raw).map_err(|e| {
                        StageError::InvalidHighWaterState {
                            path: p.display().to_string(),
                            detail: e.to_string(),
                        }
                    })?;
                    if parsed.version != STATE_VERSION {
                        return Err(StageError::InvalidHighWaterState {
                            path: p.display().to_string(),
                            detail: format!(
                                "unsupported state version {} (expected {STATE_VERSION})",
                                parsed.version
                            ),
                        }
                        .into());
                    }
                    parsed
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => StateFile {
                    version: STATE_VERSION,
                    marks: BTreeMap::new(),
                },
                Err(e) => {
                    return Err(e)
                        .wrap_err_with(|| format!("inspecting high-water state {}", p.display()))
                }
            },
            None => StateFile {
                version: STATE_VERSION,
                marks: BTreeMap::new(),
            },
        };
        Ok(Self {
            path: path.map(|p| p.to_path_buf()),
            state,
        })
    }

    /// Whether outcomes from this instance are actually persisted anywhere.
    pub fn is_persistent(&self) -> bool {
        self.path.is_some()
    }

    fn key(chain_id: u64, registry: Address) -> String {
        format!("{chain_id}:{}", registry.to_string().to_lowercase())
    }

    /// Enforce the high-water rule for a freshly authenticated anchor, then persist.
    pub fn check_and_update(
        &mut self,
        chain_id: u64,
        registry: Address,
        anchor: Mark,
        allow_rollback: bool,
    ) -> Result<Outcome> {
        let outcome = self.check_and_stage(chain_id, registry, anchor, allow_rollback)?;
        self.persist()?;
        Ok(outcome)
    }

    /// Enforce the high-water rule and stage the accepted mark in memory.
    ///
    /// The pipeline deliberately separates this from [`Self::persist`]: capture output
    /// (including `meta.json`) is written between the two operations, and the high-water
    /// state is the final writer. That ordering preserves rollback state even if a local
    /// path alias escapes the preflight checks.
    pub(crate) fn check_and_stage(
        &mut self,
        chain_id: u64,
        registry: Address,
        anchor: Mark,
        allow_rollback: bool,
    ) -> Result<Outcome> {
        let key = Self::key(chain_id, registry);
        let outcome = match self.state.marks.get(&key) {
            None => Outcome::Initialized,
            Some(stored) if anchor.block_number > stored.block_number => Outcome::Advanced,
            Some(stored)
                if anchor.block_number == stored.block_number
                    && anchor.block_hash == stored.block_hash =>
            {
                Outcome::Unchanged
            }
            Some(_) if allow_rollback => Outcome::RecoveryOverride,
            Some(stored) if anchor.block_number == stored.block_number => {
                return Err(StageError::ConflictingHash {
                    height: stored.block_number,
                    stored: stored.block_hash,
                    offered: anchor.block_hash,
                }
                .into());
            }
            Some(stored) => {
                return Err(StageError::Rollback {
                    stored: stored.block_number,
                    offered: anchor.block_number,
                }
                .into());
            }
        };
        self.state.marks.insert(key, anchor);
        Ok(outcome)
    }

    pub(crate) fn persist(&self) -> Result<()> {
        if let Some(p) = &self.path {
            let raw = serde_json::to_string_pretty(&self.state)?;
            replace_atomically(p, raw.as_bytes(), "highwater")
                .wrap_err_with(|| format!("persisting high-water state {}", p.display()))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpfile(content: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("hw.json");
        std::fs::write(&p, content).unwrap();
        (dir, p)
    }

    #[test]
    fn foreign_json_is_rejected_not_treated_as_empty() {
        // Review blocker 1: a CaptureMeta-shaped file must be a typed error.
        let (_d, p) = tmpfile(
            r#"{"capturedAtUnix":1787515000,"checkpoint":"0x00","finalizedBeaconSlot":1,"executionBlockNumber":2,"executionBlockHash":"0x00","executionStateRoot":"0x00","itemListLength":"0x1b"}"#,
        );
        let err = HighWater::load(Some(&p)).unwrap_err();
        assert!(
            err.to_string().contains("invalid high-water state"),
            "{err}"
        );
    }

    #[test]
    fn empty_object_is_rejected() {
        let (_d, p) = tmpfile("{}");
        let err = HighWater::load(Some(&p)).unwrap_err();
        assert!(
            err.to_string().contains("invalid high-water state"),
            "{err}"
        );
    }

    #[test]
    fn wrong_version_is_rejected() {
        let (_d, p) = tmpfile(r#"{"version":2,"marks":{}}"#);
        let err = HighWater::load(Some(&p)).unwrap_err();
        assert!(
            err.to_string().contains("unsupported state version 2"),
            "{err}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_state_is_rejected_not_treated_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("missing-target.json");
        let link = dir.path().join("highwater.json");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let err = HighWater::load(Some(&link)).unwrap_err();
        assert!(
            err.to_string()
                .contains("state-file leaf is a symbolic link"),
            "{err}"
        );
        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(!target.exists());
    }

    #[test]
    fn round_trip_preserves_marks_and_version() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("hw.json");
        let registry: Address = "0x54A92C21c6553a8085066311F2C8D9Db1B5e6610"
            .parse()
            .unwrap();
        let mark = Mark {
            block_number: 100,
            block_hash: B256::repeat_byte(7),
        };
        let mut hw = HighWater::load(Some(&p)).unwrap();
        assert_eq!(
            hw.check_and_update(100, registry, mark.clone(), false)
                .unwrap(),
            Outcome::Initialized
        );
        // Reload strictly and advance.
        let mut hw2 = HighWater::load(Some(&p)).unwrap();
        let next = Mark {
            block_number: 101,
            block_hash: B256::repeat_byte(8),
        };
        assert_eq!(
            hw2.check_and_update(100, registry, next, false).unwrap(),
            Outcome::Advanced
        );
    }

    #[test]
    fn final_persist_restores_state_after_static_hardlink_alias_write() {
        // Defense-in-depth ordering used by the pipeline: if a static hardlink escapes
        // path preflight, capture writes first and the atomically replaced state file is
        // still the final writer. The capture path keeps its own content after rename.
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("highwater.json");
        let capture_meta = dir.path().join("meta.json");
        let registry: Address = "0x54A92C21c6553a8085066311F2C8D9Db1B5e6610"
            .parse()
            .unwrap();

        let mut initial = HighWater::load(Some(&state)).unwrap();
        initial
            .check_and_update(
                100,
                registry,
                Mark {
                    block_number: 100,
                    block_hash: B256::repeat_byte(1),
                },
                false,
            )
            .unwrap();
        std::fs::hard_link(&state, &capture_meta).unwrap();

        let mut staged = HighWater::load(Some(&state)).unwrap();
        staged
            .check_and_stage(
                100,
                registry,
                Mark {
                    block_number: 101,
                    block_hash: B256::repeat_byte(2),
                },
                false,
            )
            .unwrap();
        let capture_body = r#"{"capturedAtUnix":1787523000}"#;
        std::fs::write(&capture_meta, capture_body).unwrap();
        staged.persist().unwrap();

        let reloaded = HighWater::load(Some(&state)).unwrap();
        let mark = reloaded
            .state
            .marks
            .get(&HighWater::key(100, registry))
            .unwrap();
        assert_eq!(mark.block_number, 101);
        assert_eq!(mark.block_hash, B256::repeat_byte(2));
        assert_eq!(std::fs::read_to_string(capture_meta).unwrap(), capture_body);
    }

    #[cfg(unix)]
    #[test]
    fn persistence_does_not_follow_legacy_temp_links() {
        // Exact regression for the former predictable `highwater.json.tmp` writer:
        // neither its victim nor the destination's file type may change.
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("highwater.json");
        let legacy_temp = state.with_extension("json.tmp");
        let victim = dir.path().join("victim.txt");
        let registry: Address = "0x54A92C21c6553a8085066311F2C8D9Db1B5e6610"
            .parse()
            .unwrap();

        let mut initial = HighWater::load(Some(&state)).unwrap();
        initial
            .check_and_update(
                100,
                registry,
                Mark {
                    block_number: 100,
                    block_hash: B256::repeat_byte(1),
                },
                false,
            )
            .unwrap();

        std::fs::write(&victim, b"must survive").unwrap();
        std::os::unix::fs::symlink(&victim, &legacy_temp).unwrap();
        let mut next = HighWater::load(Some(&state)).unwrap();
        next.check_and_update(
            100,
            registry,
            Mark {
                block_number: 101,
                block_hash: B256::repeat_byte(2),
            },
            false,
        )
        .unwrap();

        assert_eq!(std::fs::read(&victim).unwrap(), b"must survive");
        assert!(std::fs::symlink_metadata(&legacy_temp)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(!std::fs::symlink_metadata(&state)
            .unwrap()
            .file_type()
            .is_symlink());
        let reloaded = HighWater::load(Some(&state)).unwrap();
        assert_eq!(
            reloaded
                .state
                .marks
                .get(&HighWater::key(100, registry))
                .unwrap()
                .block_number,
            101
        );
    }
}
