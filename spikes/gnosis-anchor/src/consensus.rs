//! The consensus-anchor pipeline: explicit checkpoint → verified bootstrap →
//! signed sync-committee advancement → verified finality update → authenticated
//! finalized EXECUTION header (block number / hash / stateRoot / timestamp).
//!
//! All verification is done by `helios-consensus-core` generics instantiated with
//! `GnosisConsensusSpec` and the pinned Gnosis fork schedule. The Beacon API is a
//! dumb, untrusted transport. Every trust decision fails closed with a typed
//! [`StageError`]; `eyre` carries transport/decoding context only.
//!
//! TIME: live verification reads the trusted LOCAL clock immediately before every
//! signature-time check (Gnosis slots are 5 seconds — a `now` frozen at pipeline
//! start can fall behind a legitimately newer signature slot across HTTP round
//! trips; observed live by review). Offline replay uses a fixed captured time for
//! determinism. Provider-reported time is never consulted.

use std::time::{SystemTime, UNIX_EPOCH};

use alloy::primitives::B256;
use eyre::Result;
use helios_consensus_core::types::LightClientStore;
use helios_consensus_core::{
    apply_bootstrap, apply_finality_update, apply_update, calc_sync_period, verify_bootstrap,
    verify_finality_update, verify_update,
};
use tree_hash::TreeHash;

use crate::beacon::Source;
use crate::errors::StageError;
use crate::gnosis::{self, GnosisConsensusSpec};

/// Where "now" comes from. Live mode MUST use `LocalClock` (enforced at the pipeline
/// boundary); `FixedForReplay` exists only for deterministic offline replay.
#[derive(Clone, Debug)]
pub enum TimeSource {
    LocalClock,
    FixedForReplay(u64),
}

impl TimeSource {
    pub fn now_unix(&self) -> u64 {
        match self {
            TimeSource::LocalClock => SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            TimeSource::FixedForReplay(n) => *n,
        }
    }

    pub fn is_fixed(&self) -> bool {
        matches!(self, TimeSource::FixedForReplay(_))
    }
}

pub const SYNC_COMMITTEE_SIZE: u64 = 512;

/// The spike's supermajority floor, enforced ABOVE the library: helios-consensus-core
/// accepts any nonzero participation, which is insufficient for a trust anchor.
pub fn require_supermajority(participation: u64) -> Result<(), StageError> {
    if participation * 3 < SYNC_COMMITTEE_SIZE * 2 {
        return Err(StageError::InsufficientParticipation { got: participation });
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct ExecutionAnchor {
    pub block_number: u64,
    pub block_hash: B256,
    pub state_root: B256,
    pub timestamp: u64,
}

#[derive(Debug, Clone)]
pub struct AnchorReport {
    pub checkpoint: B256,
    pub bootstrap_slot: u64,
    pub checkpoint_age_secs: u64,
    pub updates_applied: usize,
    pub finalized_beacon_slot: u64,
    pub finalized_beacon_root: B256,
    pub sync_participation: u64,
    pub execution: ExecutionAnchor,
}

/// Maximum sync-committee advancement iterations (defense against a malicious endpoint
/// feeding updates that never progress).
const MAX_ADVANCE_ROUNDS: u32 = 64;
/// Beacon API cap per updates request.
const MAX_UPDATES_PER_REQUEST: u64 = 128;

pub fn derive_anchor(
    src: &Source,
    checkpoint: B256,
    max_checkpoint_age_secs: u64,
    time: &TimeSource,
) -> Result<AnchorReport> {
    let forks = gnosis::forks();

    // Stage: bootstrap — the ONLY trusted input is `checkpoint` (an explicit beacon
    // block root supplied by the operator).
    let bootstrap = src.bootstrap(checkpoint)?;
    verify_bootstrap::<GnosisConsensusSpec>(&bootstrap, checkpoint, &forks).map_err(|e| {
        StageError::Bootstrap {
            detail: format!("{e:#}"),
        }
    })?;

    let mut store: LightClientStore<GnosisConsensusSpec> = LightClientStore::default();
    apply_bootstrap(&mut store, &bootstrap);
    let bootstrap_slot = store.finalized_header.beacon().slot;

    // Stage: checkpoint staleness (explicit max age; no silent defaults).
    let checkpoint_age_secs = time
        .now_unix()
        .saturating_sub(gnosis::slot_timestamp(bootstrap_slot));
    if checkpoint_age_secs > max_checkpoint_age_secs {
        return Err(StageError::StaleCheckpoint {
            bootstrap_slot,
            age_secs: checkpoint_age_secs,
            max_age_secs: max_checkpoint_age_secs,
        }
        .into());
    }

    // Learn the target period from the (unverified, so far) finality update; the value
    // only steers which updates we REQUEST — every update is still fully verified.
    let finality = src.finality_update()?;
    let target_sig_period = calc_sync_period::<GnosisConsensusSpec>(*finality.signature_slot());

    // Stage: signed sync-committee advancement.
    let mut updates_applied = 0usize;
    let mut rounds = 0u32;
    loop {
        let store_period =
            calc_sync_period::<GnosisConsensusSpec>(store.finalized_header.beacon().slot);
        let need_advance = store_period < target_sig_period;
        let need_next_committee =
            store_period == target_sig_period && store.next_sync_committee.is_none();
        if !need_advance && !need_next_committee {
            break;
        }
        rounds += 1;
        if rounds > MAX_ADVANCE_ROUNDS {
            return Err(StageError::DidNotConverge(MAX_ADVANCE_ROUNDS).into());
        }
        let count = (target_sig_period - store_period + 1).min(MAX_UPDATES_PER_REQUEST);
        let updates = src.updates(store_period, count)?;
        if updates.is_empty() {
            return Err(StageError::NoUpdates {
                period: store_period,
                target: target_sig_period,
            }
            .into());
        }
        let before = (
            store.finalized_header.beacon().slot,
            store.next_sync_committee.is_some(),
        );
        for update in &updates {
            // Stop once advancement needs are met; later entries are not needed and
            // are simply not consumed (they were requested, but nothing obliges us
            // to verify more than the advancement requires).
            let period_now =
                calc_sync_period::<GnosisConsensusSpec>(store.finalized_header.beacon().slot);
            if period_now >= target_sig_period && store.next_sync_committee.is_some() {
                break;
            }
            let attested_slot = update.attested_header().beacon().slot;
            // Fresh trusted time for THIS verification (see module docs).
            let expected_slot = gnosis::expected_current_slot(time.now_unix());
            verify_update::<GnosisConsensusSpec>(
                update,
                expected_slot,
                &store,
                gnosis::GENESIS_VALIDATORS_ROOT,
                &forks,
            )
            .map_err(|e| StageError::UpdateRejected {
                attested_slot,
                detail: format!("{e:#}"),
            })?;
            apply_update(&mut store, update);
            updates_applied += 1;
        }
        let after = (
            store.finalized_header.beacon().slot,
            store.next_sync_committee.is_some(),
        );
        if after == before {
            return Err(StageError::NoProgress.into());
        }
    }

    // Stage: finality — full verification of the finality update (sync-committee
    // signature, finality branch, execution payload branch), then adoption. Fresh
    // trusted time again: the signature slot may legitimately be newer than any
    // earlier reading on a 5-second-slot chain.
    let expected_slot = gnosis::expected_current_slot(time.now_unix());
    verify_finality_update::<GnosisConsensusSpec>(
        &finality,
        expected_slot,
        &store,
        gnosis::GENESIS_VALIDATORS_ROOT,
        &forks,
    )
    .map_err(|e| StageError::FinalityRejected {
        detail: format!("{e:#}"),
    })?;
    let sync_participation = finality
        .sync_aggregate()
        .sync_committee_bits
        .iter()
        .filter(|b| *b)
        .count() as u64;
    require_supermajority(sync_participation)?;
    apply_finality_update(&mut store, &finality);

    let finalized_beacon_slot = store.finalized_header.beacon().slot;
    let finalized_beacon_root = store.finalized_header.beacon().tree_hash_root();

    // Stage: signed advancement requirement (brief: using the target as its own
    // checkpoint would demonstrate bootstrap acceptance, not advancement).
    if finalized_beacon_slot <= bootstrap_slot {
        return Err(StageError::NoAdvancement {
            finalized: finalized_beacon_slot,
            bootstrap: bootstrap_slot,
        }
        .into());
    }

    // Stage: authenticated finalized EXECUTION header. `is_valid_header` inside the
    // update verification has already proven execution ∈ beacon header via the
    // execution branch; post-Capella headers without it are rejected there.
    let exec = store
        .finalized_header
        .execution()
        .map_err(|_| StageError::MissingExecutionPayload)?;
    let execution = ExecutionAnchor {
        block_number: *exec.block_number(),
        block_hash: *exec.block_hash(),
        state_root: *exec.state_root(),
        timestamp: *exec.timestamp(),
    };

    Ok(AnchorReport {
        checkpoint,
        bootstrap_slot,
        checkpoint_age_secs,
        updates_applied,
        finalized_beacon_slot,
        finalized_beacon_root,
        sync_participation,
        execution,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supermajority_threshold_is_exact() {
        // 2/3 of 512 = 341.33…, so 342 is the minimum passing participation.
        assert!(matches!(
            require_supermajority(0),
            Err(StageError::InsufficientParticipation { got: 0 })
        ));
        assert!(matches!(
            require_supermajority(341),
            Err(StageError::InsufficientParticipation { got: 341 })
        ));
        require_supermajority(342).expect("342/512 is a supermajority");
        require_supermajority(512).expect("full participation passes");
    }

    #[test]
    fn fixed_time_is_fixed_and_local_clock_is_not() {
        assert!(TimeSource::FixedForReplay(7).is_fixed());
        assert_eq!(TimeSource::FixedForReplay(7).now_unix(), 7);
        assert!(!TimeSource::LocalClock.is_fixed());
    }
}
