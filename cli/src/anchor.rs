//! Header-quorum anchoring (spec §7, middle row): the same finalized execution
//! header is fetched from ≥ 2 sources with pairwise-distinct normalized origins
//! and must agree exactly. This is the labeled DEGRADED ALPHA mode — only the
//! embedded consensus light client earns "trustless" (repo invariant);
//! integrating the Gate 1 pipeline as the strict mode is tracked work. Anchor
//! strength is always reported via `MODE_LABEL`, separately from enumeration
//! scope and freshness.
//!
//! Trust rules enforced here (spec §7):
//! - every source is AUTHENTICATED against the pinned chain first: `eth_chainId`
//!   must equal the profile pin and the genesis block hash must equal the pinned
//!   `genesis_hash` — a source on the wrong chain fails closed;
//! - each returned header's hash is RECOMPUTED from its fields and must equal
//!   the reported hash, so a source cannot attach fabricated fields (timestamp
//!   included) to an agreed hash;
//! - sources must agree on (hash, stateRoot, timestamp) for the anchored height;
//! - header time is bounded in BOTH directions against a fresh local clock:
//!   stale beyond `MAX_ANCHOR_AGE_SECS` fails closed, and FUTURE skew beyond
//!   `MAX_FUTURE_SKEW_SECS` fails closed rather than counting as age zero.

use alloy::primitives::B256;
use alloy::providers::Provider;
use alloy::rpc::types::BlockId;
use eyre::{bail, eyre, Result};

use crate::profile::Profile;

/// Printed with every result that depends on this anchor mode.
pub const MODE_LABEL: &str =
    "header-quorum (degraded alpha — strict light-client mode pending integration)";

/// Maximum age of a "finalized" header before the quorum is considered unhealthy
/// (Gnosis finalizes in ~2 epochs ≈ 160 s; an hour-old finalized head means the
/// sources are stale or the chain is stalled — fail closed either way).
pub const MAX_ANCHOR_AGE_SECS: u64 = 3600;

/// Maximum tolerated FUTURE skew of an authenticated header timestamp against
/// the local clock.
pub const MAX_FUTURE_SKEW_SECS: u64 = 900;

/// Per-RPC operation deadline.
pub const RPC_DEADLINE_SECS: u64 = 30;

#[derive(Debug, Clone)]
pub struct QuorumAnchor {
    pub block_number: u64,
    pub block_hash: B256,
    pub state_root: B256,
    pub timestamp: u64,
    pub sources: usize,
}

pub async fn with_deadline<T, F>(what: &str, fut: F) -> Result<T>
where
    F: std::future::Future<Output = Result<T>>,
{
    tokio::time::timeout(std::time::Duration::from_secs(RPC_DEADLINE_SECS), fut)
        .await
        .map_err(|_| eyre!("{what}: deadline of {RPC_DEADLINE_SECS}s exceeded"))?
}

fn provider_for(rpc: &str) -> Result<impl Provider + Clone> {
    crate::transport::capped_provider(rpc)
}

/// The chain identity every source is authenticated against: chain id and
/// genesis hash (spec §7). A profile carries one; tools that pin a chain
/// without a registry profile (the Light Curate export) build one directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainPin {
    pub chain_id: u64,
    pub genesis_hash: B256,
}

impl From<&Profile> for ChainPin {
    fn from(p: &Profile) -> Self {
        Self {
            chain_id: p.chain_id,
            genesis_hash: p.genesis_hash,
        }
    }
}

/// Authenticate one RPC source against the pinned chain identity: chainId and
/// genesis hash (spec §7). Used for anchor sources AND proof RPCs.
pub async fn authenticate_source(rpc: &str, profile: &Profile) -> Result<()> {
    authenticate_source_pin(rpc, &ChainPin::from(profile)).await
}

/// `authenticate_source` against a bare chain pin.
pub async fn authenticate_source_pin(rpc: &str, pin: &ChainPin) -> Result<()> {
    let provider = provider_for(rpc)?;
    let chain_id = with_deadline(&format!("{rpc} eth_chainId"), async {
        provider
            .get_chain_id()
            .await
            .map_err(|e| eyre!("{rpc}: {e}"))
    })
    .await?;
    if chain_id != pin.chain_id {
        bail!(
            "{rpc}: serves chainId {chain_id}, profile pins {} — wrong chain",
            pin.chain_id
        );
    }
    // The genesis check is a pure IDENTITY pin: the reported hash must equal
    // the profile's pin, and NO genesis header fields are consumed — so there
    // is nothing a source could fabricate behind an honest hash. Recomputation
    // is deliberately NOT applied here: Gnosis's genesis (and its whole AuRa
    // era) uses legacy sealing whose fields don't round-trip through the
    // standard header encoding, so `hash_slow` cannot reproduce it — verified
    // empirically (reported 0x4f1dd231… vs standard-encoding 0x4590cf92…).
    // Field recomputation applies to every header whose FIELDS we trust
    // (`authenticated_header`, used for all anchor-era headers).
    // Because only the hash is consumed, the block is read as raw JSON rather
    // than through the typed header: Gnosis's AuRa genesis carries `signature`
    // and `step` fields and, from some public nodes (PublicNode, observed
    // 2026-09-07 against the live registry), a null `nonce`, which the typed
    // decoder rejects although the hash itself is present and correct.
    let genesis = with_deadline(&format!("{rpc} genesis"), async {
        provider
            .raw_request::<_, serde_json::Value>("eth_getBlockByNumber".into(), ("0x0", false))
            .await
            .map_err(|e| eyre!("{rpc}: {e}"))
    })
    .await?;
    let reported = genesis_hash_from_json(&genesis).map_err(|e| eyre!("{rpc}: {e}"))?;
    if reported != pin.genesis_hash {
        bail!(
            "{rpc}: genesis hash {} != pinned {} — wrong chain or tampered source",
            reported,
            pin.genesis_hash
        );
    }
    Ok(())
}

/// The genesis block's reported hash, and nothing else, from a raw
/// `eth_getBlockByNumber` result. Every other field is ignored on purpose:
/// legacy-sealed genesis blocks do not round-trip through the standard header
/// type, and nothing in them is trusted anyway (the pin is the hash).
fn genesis_hash_from_json(block: &serde_json::Value) -> Result<B256> {
    if block.is_null() {
        bail!("genesis block unavailable");
    }
    let hash = block
        .get("hash")
        .and_then(|h| h.as_str())
        .ok_or_else(|| eyre!("genesis block has no hash field"))?;
    hash.parse::<B256>()
        .map_err(|e| eyre!("genesis hash is not a 32-byte hex string: {e}"))
}

/// Fetch a header and AUTHENTICATE its integrity: the hash recomputed from the
/// returned fields must equal the reported hash. Returns
/// (number, reported hash, state root, timestamp).
async fn authenticated_header(rpc: &str, id: BlockId) -> Result<(u64, B256, B256, u64)> {
    let provider = provider_for(rpc)?;
    let block = with_deadline(&format!("{rpc} get_block"), async {
        provider
            .get_block(id)
            .await
            .map_err(|e| eyre!("{rpc}: {e}"))?
            .ok_or_else(|| eyre!("{rpc}: block {id:?} not available"))
    })
    .await?;
    let h = &block.header;
    let recomputed = h.inner.hash_slow();
    if recomputed != h.hash {
        bail!(
            "{rpc}: header at {} reports hash {} but its fields hash to {recomputed} — \
             fabricated header fields",
            h.number,
            h.hash
        );
    }
    Ok((h.number, h.hash, h.state_root, h.timestamp))
}

fn unix_now() -> Result<u64> {
    // Fresh local time on EVERY check (Gate 1 lesson: never cache "now").
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| eyre!("system clock before epoch: {e}"))?
        .as_secs())
}

fn check_time_bounds(timestamp: u64) -> Result<()> {
    let now = unix_now()?;
    if timestamp > now.saturating_add(MAX_FUTURE_SKEW_SECS) {
        bail!(
            "quorum header timestamp {timestamp} is {}s in the FUTURE (> {MAX_FUTURE_SKEW_SECS}s \
             skew bound) — failing closed",
            timestamp - now
        );
    }
    let age = now.saturating_sub(timestamp);
    if age > MAX_ANCHOR_AGE_SECS {
        bail!(
            "quorum finalized header is {age}s old (> {MAX_ANCHOR_AGE_SECS}s) — \
             stale sources or stalled finality; failing closed"
        );
    }
    Ok(())
}

/// Agree on the CURRENT finalized header across all quorum sources: authenticate
/// every source's chain identity, take the minimum finalized height any source
/// reports, then require every source to serve an identical authenticated
/// header (hash, stateRoot, timestamp) at that height.
pub async fn finalized_quorum(profile: &Profile) -> Result<QuorumAnchor> {
    finalized_quorum_rpcs(&profile.anchor_rpcs, &ChainPin::from(profile)).await
}

/// `finalized_quorum` over an explicit source list and a bare chain pin.
pub async fn finalized_quorum_rpcs(rpcs: &[String], pin: &ChainPin) -> Result<QuorumAnchor> {
    if rpcs.len() < 2 {
        bail!("header quorum needs at least 2 RPC endpoints");
    }
    let mut min_number = u64::MAX;
    for rpc in rpcs {
        authenticate_source_pin(rpc, pin).await?;
        let (number, _, _, _) = authenticated_header(rpc, BlockId::finalized()).await?;
        min_number = min_number.min(number);
    }
    if min_number == u64::MAX || min_number == 0 {
        bail!("no usable finalized height from quorum sources");
    }
    let (hash, state_root, timestamp) = quorum_at_rpcs(rpcs, pin, min_number, false).await?;
    check_time_bounds(timestamp)?;
    Ok(QuorumAnchor {
        block_number: min_number,
        block_hash: hash,
        state_root,
        timestamp,
        sources: rpcs.len(),
    })
}

/// Authenticate a SPECIFIC block height across all quorum sources (used to check
/// a provider snapshot's declared anchor): every source must serve an identical
/// authenticated (hash, stateRoot, timestamp) for that height, and the height
/// must be at or below every source's finalized head.
pub async fn quorum_at(profile: &Profile, number: u64) -> Result<(B256, B256, u64)> {
    quorum_at_rpcs(&profile.anchor_rpcs, &ChainPin::from(profile), number, true).await
}

async fn quorum_at_rpcs(
    rpcs: &[String],
    pin: &ChainPin,
    number: u64,
    authenticate: bool,
) -> Result<(B256, B256, u64)> {
    if rpcs.len() < 2 {
        bail!("header quorum needs at least 2 RPC endpoints");
    }
    let mut agreed: Option<(B256, B256, u64)> = None;
    for rpc in rpcs {
        if authenticate {
            authenticate_source_pin(rpc, pin).await?;
        }
        let (fin_number, _, _, _) = authenticated_header(rpc, BlockId::finalized()).await?;
        if number > fin_number {
            bail!("{rpc}: block {number} is beyond its finalized head {fin_number}");
        }
        let (got_number, hash, state_root, timestamp) =
            authenticated_header(rpc, BlockId::number(number)).await?;
        if got_number != number {
            bail!("{rpc}: asked for block {number}, got {got_number}");
        }
        match &agreed {
            None => agreed = Some((hash, state_root, timestamp)),
            Some((h, s, t)) => {
                if *h != hash || *s != state_root || *t != timestamp {
                    bail!(
                        "header-quorum DISAGREEMENT at block {number}: {rpc} serves \
                         hash {hash} / stateRoot {state_root} / time {timestamp}, another \
                         source served {h} / {s} / {t} — refusing this anchor"
                    );
                }
            }
        }
    }
    agreed.ok_or_else(|| eyre!("empty quorum"))
}

/// Wrap an authenticated historical height as a QuorumAnchor (for provider
/// snapshots anchored below the current finalized head). Applies the same time
/// bounds as the live quorum.
pub async fn quorum_anchor_at(profile: &Profile, number: u64) -> Result<QuorumAnchor> {
    let (hash, state_root, timestamp) = quorum_at(profile, number).await?;
    if timestamp > unix_now()?.saturating_add(MAX_FUTURE_SKEW_SECS) {
        bail!("anchored header timestamp is beyond the future-skew bound — failing closed");
    }
    Ok(QuorumAnchor {
        block_number: number,
        block_hash: hash,
        state_root,
        timestamp,
        sources: profile.anchor_rpcs.len(),
    })
}

#[cfg(test)]
mod genesis_tests {
    use super::genesis_hash_from_json;
    use alloy::primitives::b256;

    /// Gnosis's AuRa genesis as PublicNode serves it: legacy sealing fields
    /// (`signature`, `step`) and a null `nonce`, which the typed header
    /// decoder rejects. Only the hash is read.
    #[test]
    fn reads_the_hash_of_a_legacy_sealed_genesis_with_a_null_nonce() {
        let block: serde_json::Value = serde_json::from_str(
            r#"{"difficulty":"0x20000","extraData":"0x","gasLimit":"0x989680","gasUsed":"0x0",
                "hash":"0x4f1dd23188aab3a76b463e4af801b52b1248ef073c648cbdc4c9333d3da79756",
                "miner":"0x0000000000000000000000000000000000000000","nonce":null,"number":"0x0",
                "parentHash":"0x0000000000000000000000000000000000000000000000000000000000000000",
                "signature":"0x00","step":0,"stateRoot":"0x40cf4430ecaa733787d1a65154a3b9efb560c95d9e324a23b97f0609b539133b",
                "timestamp":"0x0","transactions":[],"uncles":[]}"#,
        )
        .unwrap();
        assert_eq!(
            genesis_hash_from_json(&block).unwrap(),
            b256!("4f1dd23188aab3a76b463e4af801b52b1248ef073c648cbdc4c9333d3da79756")
        );
    }

    #[test]
    fn rejects_a_missing_block_and_a_malformed_hash() {
        assert!(genesis_hash_from_json(&serde_json::Value::Null).is_err());
        assert!(genesis_hash_from_json(&serde_json::json!({"number": "0x0"})).is_err());
        assert!(genesis_hash_from_json(&serde_json::json!({"hash": "0x1234"})).is_err());
    }
}
