//! Bounded error type for trust-stage failures (review fix 10).
//!
//! Every failure that DECIDES trust — a rejected proof, a stale checkpoint, an
//! undecodable status, a high-water violation — is one of these variants. Transport,
//! decoding and CLI plumbing continue to propagate through `eyre` for context; the
//! boundary is deliberate and documented in the results document: `eyre` never decides
//! anything, it only carries one of these upward.

use alloy::primitives::{Address, B256, U256};

#[derive(Debug, thiserror::Error)]
pub enum StageError {
    #[error("invalid live configuration: --state-file is required in live mode (ephemeral rollback protection would misreport a security state)")]
    LiveStateFileRequired,

    #[error("invalid live configuration: fixed replay time is forbidden in live mode; live verification must read the trusted local clock")]
    LiveFixedTimeForbidden,

    #[error("invalid configuration: state file {state_file} is equal to or inside the capture directory {capture_dir}; captured responses would overwrite rollback-protection state")]
    StateFileInsideCaptureDir {
        state_file: String,
        capture_dir: String,
    },

    #[error("invalid high-water state at {path}: {detail}; refusing to treat unrecognized state as empty")]
    InvalidHighWaterState { path: String, detail: String },

    #[error("cannot resolve path {path} for aliasing checks: {detail}")]
    PathResolution { path: String, detail: String },

    #[error("endpoint genesis mismatch (got time {got_time}, root {got_root}); expected Gnosis mainnet (time {want_time}, root {want_root})")]
    GenesisMismatch {
        got_time: u64,
        got_root: B256,
        want_time: u64,
        want_root: B256,
    },

    #[error("unsupported light-client fork version '{version}' (allow-listed: {allowed}); refusing to decode an unknown schema")]
    UnsupportedForkVersion { version: String, allowed: String },

    #[error("bootstrap verification failed against the supplied checkpoint: {detail}")]
    Bootstrap { detail: String },

    #[error("stale checkpoint: bootstrap slot {bootstrap_slot} is {age_secs}s old, exceeding --max-checkpoint-age {max_age_secs}s")]
    StaleCheckpoint {
        bootstrap_slot: u64,
        age_secs: u64,
        max_age_secs: u64,
    },

    #[error("light-client update rejected (attested slot {attested_slot}): {detail}")]
    UpdateRejected { attested_slot: u64, detail: String },

    #[error("endpoint returned no light-client updates for period {period} (cannot advance to period {target})")]
    NoUpdates { period: u64, target: u64 },

    #[error("light-client updates made no progress; refusing to loop")]
    NoProgress,

    #[error("sync-committee advancement did not converge within {0} rounds")]
    DidNotConverge(u32),

    #[error("finality update verification failed: {detail}")]
    FinalityRejected { detail: String },

    #[error(
        "insufficient sync-committee participation for finality: {got}/512 (< 2/3 supermajority)"
    )]
    InsufficientParticipation { got: u64 },

    #[error("no signed advancement: finalized slot {finalized} does not advance past the checkpoint bootstrap slot {bootstrap}; supply an earlier checkpoint")]
    NoAdvancement { finalized: u64, bootstrap: u64 },

    #[error("finalized header lacks an execution payload header (pre-Capella?)")]
    MissingExecutionPayload,

    #[error("proof is for {got}, profile targets {want}")]
    WrongAddress { got: Address, want: Address },

    #[error("account proof does not verify against the authenticated state root: {detail}")]
    AccountProof { detail: String },

    #[error("runtime code hash mismatch: proven {proven}, locally pinned {pinned}")]
    CodeHashMismatch { proven: B256, pinned: B256 },

    #[error("provider response missing proof for locally derived slot {slot} ({meaning})")]
    MissingSlotProof { slot: B256, meaning: String },

    #[error("storage proof invalid for slot {slot} ({meaning}): {detail}")]
    StorageProof {
        slot: B256,
        meaning: String,
        detail: String,
    },

    #[error("undecodable item status value {value} (expected 0..=3); failing closed")]
    UndecodableStatus { value: U256 },

    #[error("high-water violation: anchor rollback from block {stored} to {offered}; refusing without --allow-rollback")]
    Rollback { stored: u64, offered: u64 },

    #[error("high-water violation: conflicting block hash at height {height} (stored {stored}, offered {offered}); refusing without --allow-rollback")]
    ConflictingHash {
        height: u64,
        stored: B256,
        offered: B256,
    },
}
