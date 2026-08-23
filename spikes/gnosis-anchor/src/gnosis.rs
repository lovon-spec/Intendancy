//! Gnosis Chain consensus parameters, pinned from canonical sources.
//!
//! Provenance (recorded in docs/spikes/gnosis-anchor-results.md):
//! - gnosischain/configs @ e542d132340e68fd7922149b145a0d361e1c87d4 (mainnet/config.yaml)
//! - live cross-check: https://rpc-gbc.gnosischain.com/eth/v1/config/spec and /eth/v1/beacon/genesis
//!   (captured as fixtures; the pinned constants below are authoritative, the live values are
//!   only cross-checked against them — never the other way around).
//!
//! The values below are Gnosis mainnet ("gnosis" preset), NOT Ethereum mainnet:
//! 5-second slots, 16 slots/epoch, 512 epochs per sync-committee period,
//! MAX_WITHDRAWALS_PER_PAYLOAD = 8.

use alloy::primitives::{b256, B256};
use helios_consensus_core::consensus_spec::ConsensusSpec;
use helios_consensus_core::types::{Fork, Forks};
use serde::{Deserialize, Serialize};

/// Gnosis mainnet consensus spec ("gnosis" preset).
///
/// This is the entire "Gnosis support" delta against upstream helios-consensus-core:
/// the crate's verification functions are generic over `ConsensusSpec`, so injecting
/// this type (plus the `forks()` data below) requires no fork of upstream.
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct GnosisConsensusSpec;

impl ConsensusSpec for GnosisConsensusSpec {
    type MaxProposerSlashings = typenum::U16;
    type MaxAttesterSlashings = typenum::U2;
    type MaxAttesterSlashingsElectra = typenum::U1;
    type MaxAttestations = typenum::U128;
    type MaxAttestationsElectra = typenum::U8;
    type MaxCommitteesPerSlot = typenum::U64;
    type MaxValidatorsPerSlot = typenum::U131072;
    type MaxDeposits = typenum::U16;
    type MaxVoluntaryExits = typenum::U16;
    type MaxBlsToExecutionChanged = typenum::U16;
    type MaxBlobKzgCommitments = typenum::U4096;
    /// Gnosis preset: MAX_WITHDRAWALS_PER_PAYLOAD = 8 (mainnet: 16).
    type MaxWithdrawals = typenum::U8;
    type MaxValidatorsPerCommittee = typenum::U2048;
    /// Gnosis preset: SLOTS_PER_EPOCH = 16 (mainnet: 32).
    type SlotsPerEpoch = typenum::U16;
    /// Gnosis preset: EPOCHS_PER_SYNC_COMMITTEE_PERIOD = 512 (mainnet: 256).
    type EpochsPerSyncCommitteePeriod = typenum::U512;
    type SyncCommitteeSize = typenum::U512;
    type MaxWithdrawalRequests = typenum::U16;
    type MaxDepositRequests = typenum::U8192;
    type MaxConsolidationRequests = typenum::U2;
}

/// Gnosis beacon genesis time (actual genesis, from /eth/v1/beacon/genesis; note this is
/// later than MIN_GENESIS_TIME + GENESIS_DELAY because genesis triggered on validator count).
pub const GENESIS_TIME: u64 = 1_638_993_340;

/// Gnosis genesis validators root (domain separation for sync-committee signatures).
pub const GENESIS_VALIDATORS_ROOT: B256 =
    b256!("f5dcb5564e829aab27264b9becd5dfaa017085611224cb3036f573368dbb9d47");

/// Gnosis preset: SECONDS_PER_SLOT = 5 (mainnet: 12). Never use an upstream helper that
/// hardcodes mainnet slot timing.
pub const SECONDS_PER_SLOT: u64 = 5;

pub const CHAIN_ID: u64 = 100;

/// Gnosis mainnet fork schedule (epochs + versions) through Fulu.
/// GLOAS is unscheduled (`FAR_FUTURE_EPOCH`) as of the pinned configs revision and is
/// deliberately absent: an update from an unknown fork must fail closed at decode time.
pub fn forks() -> Forks {
    fn f(epoch: u64, version: [u8; 4]) -> Fork {
        Fork {
            epoch,
            fork_version: version.into(),
        }
    }
    Forks {
        genesis: f(0, [0x00, 0x00, 0x00, 0x64]),
        altair: f(512, [0x01, 0x00, 0x00, 0x64]),
        bellatrix: f(385_536, [0x02, 0x00, 0x00, 0x64]),
        capella: f(648_704, [0x03, 0x00, 0x00, 0x64]),
        deneb: f(889_856, [0x04, 0x00, 0x00, 0x64]),
        electra: f(1_337_856, [0x05, 0x00, 0x00, 0x64]),
        fulu: f(1_714_688, [0x06, 0x00, 0x00, 0x64]),
    }
}

/// Wall-clock timestamp of a Gnosis slot.
pub fn slot_timestamp(slot: u64) -> u64 {
    GENESIS_TIME + slot * SECONDS_PER_SLOT
}

/// Expected current slot at `now_unix`, using Gnosis 5-second slots.
/// (Implemented locally: upstream `expected_current_slot` must not be assumed to use
/// the correct slot duration for Gnosis; see results doc.)
pub fn expected_current_slot(now_unix: u64) -> u64 {
    now_unix.saturating_sub(GENESIS_TIME) / SECONDS_PER_SLOT
}
