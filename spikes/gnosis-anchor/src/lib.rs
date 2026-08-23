//! Gnosis finalized-state anchor spike (Implementation Brief 0001). Non-production.
//!
//! Verification chain:
//! explicit checkpoint → light-client bootstrap/updates/finality (helios-consensus-core,
//! instantiated with the Gnosis preset) → authenticated finalized execution stateRoot →
//! EIP-1186 account/storage proofs (alloy-trie) → pinned-codehash Classic GTCR storage.

mod atomic_file;
pub mod beacon;
pub mod consensus;
pub mod errors;
pub mod gnosis;
pub mod highwater;
pub mod pipeline;
pub mod profile;
pub mod proof;
pub mod report;
