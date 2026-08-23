//! End-to-end pipeline shared by the CLI and the test suites.
//!
//! The LIVE-MODE INVARIANTS live HERE, at the library boundary (review blocker 2),
//! not in the CLI: live verification requires persistent high-water state, requires
//! the trusted local clock, and refuses a state file that captured responses could
//! overwrite. `validate()` runs before any network access.

use std::path::{Path, PathBuf};

use alloy::primitives::B256;
use eyre::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::beacon::{write_capture_file, Source};
use crate::consensus::{derive_anchor, TimeSource};
use crate::errors::StageError;
use crate::highwater::{HighWater, Mark};
use crate::profile::Profile;
use crate::proof::verify_response;
use crate::report::Report;

pub struct PipelineConfig {
    pub source: Source,
    pub checkpoint: B256,
    pub profile: Profile,
    pub max_checkpoint_age_secs: u64,
    pub time: TimeSource,
    pub state_file: Option<PathBuf>,
    pub allow_rollback: bool,
    pub anchor_source_label: String,
}

/// Written next to captured fixtures so offline replay is fully deterministic
/// (same `now`, same expected outputs).
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CaptureMeta {
    pub captured_at_unix: u64,
    pub checkpoint: B256,
    pub finalized_beacon_slot: u64,
    pub execution_block_number: u64,
    pub execution_block_hash: B256,
    pub execution_state_root: B256,
    pub item_list_length: Option<String>,
}

/// Library-boundary invariants; MUST hold before any network access. Validation may
/// create the configured output directories so their real filesystem identities can
/// be compared before a response is written.
fn validate(cfg: &PipelineConfig) -> Result<(), StageError> {
    if let Source::Live { capture_dir, .. } = &cfg.source {
        let state_file = cfg
            .state_file
            .as_deref()
            .ok_or(StageError::LiveStateFileRequired)?;
        if cfg.time.is_fixed() {
            return Err(StageError::LiveFixedTimeForbidden);
        }
        if let Some(capture) = capture_dir {
            if path_is_within(state_file, capture)? {
                return Err(StageError::StateFileInsideCaptureDir {
                    state_file: state_file.display().to_string(),
                    capture_dir: capture.display().to_string(),
                });
            }
        } else {
            // Persistent rollback state must fail closed even when response capture is
            // disabled. In particular, a dangling leaf symlink must not look like a
            // first run merely because its target is temporarily unavailable.
            let cwd = std::env::current_dir().map_err(|e| {
                path_error(Path::new("."), format!("reading current directory: {e}"))
            })?;
            prepare_state_file(state_file, &cwd)?;
        }
    }
    Ok(())
}

fn path_error(path: &Path, detail: impl Into<String>) -> StageError {
    StageError::PathResolution {
        path: path.display().to_string(),
        detail: detail.into(),
    }
}

fn absolute_from(path: &Path, cwd: &Path) -> Result<PathBuf, StageError> {
    if path.as_os_str().is_empty() {
        return Err(path_error(path, "path is empty"));
    }
    Ok(if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    })
}

/// Prepare and canonicalize a capture directory before network access. Creating this
/// directory is a normal consequence of `--capture-dir`; doing it during validation is
/// what prevents a dangling symlink or filesystem-normalized alias from becoming active
/// only after the containment check.
fn prepare_capture_dir(path: &Path, cwd: &Path) -> Result<PathBuf, StageError> {
    let absolute = absolute_from(path, cwd)?;
    std::fs::create_dir_all(&absolute)
        .map_err(|e| path_error(path, format!("creating capture directory: {e}")))?;
    std::fs::canonicalize(&absolute)
        .map_err(|e| path_error(path, format!("canonicalizing capture directory: {e}")))
}

/// Resolve an existing or future state file against a canonical, prepared parent.
/// Only the leaf may be absent. A dangling leaf symlink is rejected explicitly instead
/// of being mistaken for a nonexistent regular file.
fn prepare_state_file(path: &Path, cwd: &Path) -> Result<PathBuf, StageError> {
    let absolute = absolute_from(path, cwd)?;
    let name = absolute
        .file_name()
        .ok_or_else(|| path_error(path, "state file has no file name"))?
        .to_os_string();
    let parent = absolute
        .parent()
        .ok_or_else(|| path_error(path, "state file has no parent directory"))?;

    std::fs::create_dir_all(parent)
        .map_err(|e| path_error(path, format!("creating state-file parent: {e}")))?;
    let real_parent = std::fs::canonicalize(parent)
        .map_err(|e| path_error(path, format!("canonicalizing state-file parent: {e}")))?;

    match std::fs::symlink_metadata(&absolute) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(path_error(
                    path,
                    "state-file leaf is a symbolic link; use a regular file path",
                ));
            }
            let resolved = std::fs::canonicalize(&absolute).map_err(|e| {
                path_error(path, format!("canonicalizing existing state file: {e}"))
            })?;
            Ok(resolved)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(real_parent.join(name)),
        Err(e) => Err(path_error(
            path,
            format!("reading state-file metadata: {e}"),
        )),
    }
}

/// Containment after preparing both destinations and resolving them through the real
/// filesystem. Every capture file is atomically replaced rather than opened for
/// writing, and the high-water file is persisted as the final writer, so static link
/// aliases cannot mutate rollback state.
/// Post-validation path replacement remains part of the trusted-local-filesystem
/// boundary of this non-production spike.
fn path_is_within(candidate: &Path, dir: &Path) -> Result<bool, StageError> {
    let cwd = std::env::current_dir()
        .map_err(|e| path_error(Path::new("."), format!("reading current directory: {e}")))?;
    let dir = prepare_capture_dir(dir, &cwd)?;
    let candidate = prepare_state_file(candidate, &cwd)?;
    Ok(candidate == dir || candidate.starts_with(&dir))
}

/*
 * Keep path preparation above deliberately small and standard-library-only. It is not
 * a general secure-open primitive; production code should use directory handles and
 * platform-specific no-follow/openat semantics to close local filesystem races.
 */
pub fn run(cfg: &PipelineConfig) -> (Report, Result<()>) {
    let mut report = Report::new(
        cfg.profile.chain_id,
        cfg.anchor_source_label.clone(),
        cfg.checkpoint,
    );
    let outcome = run_inner(cfg, &mut report);
    (report, outcome)
}

fn run_inner(cfg: &PipelineConfig, report: &mut Report) -> Result<()> {
    // Stage -1: configuration invariants (before any network I/O). Output directories
    // may be created here so aliases are resolved before response capture begins.
    if let Err(e) = validate(cfg) {
        report.stage_fail("config", e.to_string());
        return Err(e.into());
    }
    report.stage_ok("config", "live-mode invariants satisfied");

    // Stage 0: the endpoint must actually be Gnosis mainnet.
    match cfg.source.check_genesis() {
        Ok(()) => report.stage_ok(
            "genesis-crosscheck",
            "endpoint genesis matches pinned Gnosis values",
        ),
        Err(e) => {
            report.stage_fail("genesis-crosscheck", format!("{e:#}"));
            return Err(e);
        }
    }

    // Stages 1–4: checkpoint → consensus finality → authenticated execution header.
    let anchor = match derive_anchor(
        &cfg.source,
        cfg.checkpoint,
        cfg.max_checkpoint_age_secs,
        &cfg.time,
    ) {
        Ok(a) => {
            report.stage_ok(
                "consensus-anchor",
                format!(
                    "finalized beacon slot {} via {} verified update(s); participation {}/512",
                    a.finalized_beacon_slot, a.updates_applied, a.sync_participation
                ),
            );
            report.record_anchor(&a);
            a
        }
        Err(e) => {
            report.stage_fail("consensus-anchor", format!("{e:#}"));
            return Err(e);
        }
    };

    // Stage 5: EIP-1186 proof at the exact finalized execution block.
    let slots: Vec<B256> = cfg.profile.slots.iter().map(|s| s.derive()).collect();
    let proof_resp =
        match cfg
            .source
            .get_proof(cfg.profile.address, &slots, anchor.execution.block_number)
        {
            Ok(p) => p,
            Err(e) => {
                report.stage_fail("proof-fetch", format!("{e:#}"));
                return Err(e);
            }
        };
    report.stage_ok(
        "proof-fetch",
        format!(
            "eth_getProof for {} at block {} ({} storage slot(s))",
            cfg.profile.address,
            anchor.execution.block_number,
            slots.len()
        ),
    );

    let verified = match verify_response(&proof_resp, anchor.execution.state_root, &cfg.profile) {
        Ok(v) => {
            report.stage_ok(
                "account-proof",
                format!(
                    "account verified against stateRoot; storageRoot {}",
                    v.storage_root
                ),
            );
            report.stage_ok(
                "codehash-pin",
                format!("runtime codehash matches pinned {}", v.code_hash),
            );
            report.stage_ok(
                "storage-proof",
                format!("{} slot(s) verified", v.storage.len()),
            );
            v
        }
        Err(e) => {
            report.stage_fail("proof-verify", format!("{e:#}"));
            return Err(e.into());
        }
    };
    report.record_account(cfg.profile.address, &verified);

    // Stage 6: rollback resistance. First validate and stage the monotonic update in
    // memory. Capture metadata is written next, and the high-water file is persisted
    // LAST. The ordering is defense in depth: even a static filesystem alias missed by
    // preflight cannot leave a successful run with its rollback state overwritten.
    let mut hw = HighWater::load(cfg.state_file.as_deref())?;
    let outcome = match hw.check_and_stage(
        cfg.profile.chain_id,
        cfg.profile.address,
        Mark {
            block_number: anchor.execution.block_number,
            block_hash: anchor.execution.block_hash,
        },
        cfg.allow_rollback,
    ) {
        Ok(outcome) => outcome,
        Err(e) => {
            report.stage_fail("high-water", format!("{e:#}"));
            return Err(e);
        }
    };

    if let Source::Live {
        capture_dir: Some(dir),
        ..
    } = &cfg.source
    {
        let write_result = (|| -> Result<()> {
            let meta = capture_meta(cfg, report)
                .ok_or_else(|| eyre::eyre!("verified report is missing capture metadata fields"))?;
            write_capture_file(
                dir,
                "meta.json",
                serde_json::to_string_pretty(&meta)?.as_bytes(),
            )
            .wrap_err("writing capture meta.json")?;
            Ok(())
        })();
        if let Err(e) = write_result {
            report.stage_fail("capture-meta", format!("{e:#}"));
            return Err(e);
        }
    }

    if let Err(e) = hw.persist() {
        report.stage_fail("high-water", format!("{e:#}"));
        return Err(e);
    }

    report.high_water = Some(outcome);
    // Honest security grade: an unpersisted mark protects nothing beyond this
    // process. Live mode requires a state file at the library boundary; this
    // label covers offline/test callers that run without one.
    let persistence = if cfg.state_file.is_some() {
        "persisted"
    } else {
        "EPHEMERAL (no state file — not persisted)"
    };
    report.stage_ok("high-water", format!("{outcome:?} [{persistence}]"));

    Ok(())
}

/// Build capture metadata after proof verification and before the final high-water
/// persistence.
pub fn capture_meta(cfg: &PipelineConfig, report: &Report) -> Option<CaptureMeta> {
    let fb = report.finalized_beacon.as_ref()?;
    let ex = report.execution.as_ref()?;
    let item_list_length = report
        .storage
        .as_ref()
        .and_then(|s| s.first())
        .map(|s| format!("{:#x}", s.value));
    Some(CaptureMeta {
        captured_at_unix: cfg.time.now_unix(),
        checkpoint: cfg.checkpoint,
        finalized_beacon_slot: fb.slot,
        execution_block_number: ex.block_number,
        execution_block_hash: ex.block_hash,
        execution_state_root: ex.state_root,
        item_list_length,
    })
}
