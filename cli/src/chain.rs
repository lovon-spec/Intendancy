//! Consumer-side chain access. Two roles, both against UNTRUSTED RPCs:
//!
//! - **Snapshot self-generation** (`generate_snapshot`): the RPC plays the
//!   provider role from the spec — rows via view calls, proofs via
//!   `eth_getProof` — and everything it returns is then verified locally against
//!   the quorum-authenticated anchor. A lying RPC fails verification, it cannot
//!   corrupt state.
//! - **Fresh point check** (`point_check`, spec §8): account + one status slot
//!   proven at the latest finalized quorum anchor. Point results are
//!   "verified, non-exhaustive" by construction (spec §10) — they answer
//!   freshness for one item, never completeness.

use std::collections::BTreeMap;

use alloy::primitives::{keccak256, Bytes, B256, U256};
use alloy::providers::Provider;
use alloy::rpc::types::BlockId;
use eyre::{bail, eyre, Result};

use crate::anchor::QuorumAnchor;
use crate::profile::Profile;
use crate::snapshot::{
    item_list_slot, item_status_slot, verify_account, verify_slot, AccountFields, Anchor, Binding,
    Limits, Proofs, Row, SlotProof, Snapshot, ITEM_LIST_SLOT, META_EVIDENCE_UPDATES_SLOT,
    SNAPSHOT_VERSION,
};

mod abi {
    use alloy::sol;

    sol! {
        #[sol(rpc)]
        interface IGTCR {
            function itemCount() external view returns (uint256);
            function itemList(uint256 _index) external view returns (bytes32);
            function getItemInfo(bytes32 _itemID)
                external view returns (bytes data, uint8 status, uint256 numberOfRequests);
        }

        #[sol(rpc)]
        interface IMulticall3 {
            struct Call3 { address target; bool allowFailure; bytes callData; }
            struct Result { bool success; bytes returnData; }
            function aggregate3(Call3[] calldata calls)
                external payable returns (Result[] memory returnData);
        }
    }
}
use abi::{IMulticall3, IGTCR};

/// Canonical Multicall3 deployment (same address on Gnosis and the anvil fork).
const MULTICALL3: alloy::primitives::Address =
    alloy::primitives::address!("cA11bde05977b3631167028862bE2a173976CA11");
/// Subcalls per aggregate3 batch.
const MULTICALL_CHUNK: usize = 400;

fn http_provider(rpc_url: &str) -> Result<impl Provider + Clone> {
    crate::transport::capped_provider(rpc_url)
}

/// A Multicall batch MUST return exactly one result per subcall BEFORE any
/// indexing/decoding — a malicious extra or missing result otherwise corrupts
/// row alignment or panics (review finding).
pub fn check_batch_cardinality(calls: usize, results: usize) -> Result<()> {
    if calls != results {
        bail!("multicall returned {results} results for {calls} subcalls (malformed batch)");
    }
    Ok(())
}

/// Build a provider for an UNTRUSTED proof/data RPC after authenticating its
/// chain identity (chainId + pinned genesis hash, spec §7).
async fn checked_provider(rpc_url: &str, profile: &Profile) -> Result<impl Provider + Clone> {
    crate::anchor::authenticate_source(rpc_url, profile).await?;
    http_provider(rpc_url)
}

/// Self-generate a complete snapshot from an untrusted RPC at the authenticated
/// anchor. The caller MUST still run `snapshot::verify` on the result — this
/// function trusts nothing it fetched.
pub async fn generate_snapshot(
    rpc_url: &str,
    profile: &Profile,
    anchor: &QuorumAnchor,
    limits: &Limits,
) -> Result<Snapshot> {
    use alloy::sol_types::SolCall;
    let provider = checked_provider(rpc_url, profile).await?;
    let at: BlockId = anchor.block_number.into();
    let gtcr = IGTCR::new(profile.registry, provider.clone());
    let mc = IMulticall3::new(MULTICALL3, provider.clone());

    let count: U256 = crate::anchor::with_deadline(&format!("{rpc_url} itemCount"), async {
        gtcr.itemCount()
            .block(at)
            .call()
            .await
            .map_err(|e| eyre!("{e}"))
    })
    .await?;
    let count: u64 = count
        .try_into()
        .map_err(|_| eyre!("itemCount {count} out of range"))?;
    if count > limits.max_items {
        bail!(
            "registry reports {count} items, over the {} cap",
            limits.max_items
        );
    }

    // Batched enumeration via Multicall3 aggregate3 (allowFailure = false, so a
    // single failed subcall fails the whole batch — fail closed), all pinned to
    // the anchor block.
    let run_batch = |calldatas: Vec<Bytes>| {
        let mc = mc.clone();
        async move {
            let calls: Vec<IMulticall3::Call3> = calldatas
                .into_iter()
                .map(|cd| IMulticall3::Call3 {
                    target: profile.registry,
                    allowFailure: false,
                    callData: cd,
                })
                .collect();
            let n_calls = calls.len();
            let results = crate::anchor::with_deadline(&format!("{rpc_url} aggregate3"), async {
                mc.aggregate3(calls)
                    .block(at)
                    .call()
                    .await
                    .map_err(|e| eyre!("{e}"))
            })
            .await?;
            check_batch_cardinality(n_calls, results.len())?;
            Ok::<Vec<IMulticall3::Result>, eyre::Report>(results)
        }
    };

    let mut ids: Vec<B256> = Vec::with_capacity(count as usize);
    for chunk_start in (0..count).step_by(MULTICALL_CHUNK) {
        let end = (chunk_start + MULTICALL_CHUNK as u64).min(count);
        let calldatas: Vec<Bytes> = (chunk_start..end)
            .map(|i| {
                IGTCR::itemListCall {
                    _index: U256::from(i),
                }
                .abi_encode()
                .into()
            })
            .collect();
        for r in run_batch(calldatas).await? {
            let id = IGTCR::itemListCall::abi_decode_returns(&r.returnData)
                .map_err(|e| eyre!("itemList decode: {e}"))?;
            ids.push(id);
        }
    }
    if ids.len() as u64 != count {
        bail!(
            "multicall returned {} itemList entries for count {count}",
            ids.len()
        );
    }

    let mut rows = Vec::with_capacity(count as usize);
    for (chunk_start, id_chunk) in ids.chunks(MULTICALL_CHUNK).enumerate() {
        let calldatas: Vec<Bytes> = id_chunk
            .iter()
            .map(|id| IGTCR::getItemInfoCall { _itemID: *id }.abi_encode().into())
            .collect();
        for (j, r) in run_batch(calldatas).await?.into_iter().enumerate() {
            let info = IGTCR::getItemInfoCall::abi_decode_returns(&r.returnData)
                .map_err(|e| eyre!("getItemInfo decode: {e}"))?;
            let i = (chunk_start * MULTICALL_CHUNK + j) as u64;
            rows.push(Row {
                index: i,
                item_id: id_chunk[j],
                status: info.status,
                descriptor: info.data,
            });
        }
    }

    // Slot set: length + policy-immutability counter + every itemList[i] +
    // every status slot.
    let mut slots: Vec<B256> = Vec::with_capacity(2 + 2 * rows.len());
    slots.push(B256::from(U256::from(ITEM_LIST_SLOT)));
    slots.push(B256::from(U256::from(META_EVIDENCE_UPDATES_SLOT)));
    for row in &rows {
        slots.push(item_list_slot(row.index));
        slots.push(item_status_slot(row.item_id));
    }

    // Harvest proofs in chunks into the deduplicated node store.
    const CHUNK: usize = 250;
    let mut nodes: BTreeMap<B256, Bytes> = BTreeMap::new();
    let mut slot_proofs: Vec<SlotProof> = Vec::with_capacity(slots.len());
    let mut account: Option<(AccountFields, Vec<Bytes>)> = None;
    for chunk in slots.chunks(CHUNK) {
        let resp = crate::anchor::with_deadline(&format!("{rpc_url} eth_getProof"), async {
            provider
                .get_proof(profile.registry, chunk.to_vec())
                .block_id(at)
                .await
                .map_err(|e| eyre!("{e}"))
        })
        .await?;
        if account.is_none() {
            account = Some((
                AccountFields {
                    nonce: resp.nonce,
                    balance: resp.balance,
                    storage_root: resp.storage_hash,
                    code_hash: resp.code_hash,
                },
                resp.account_proof.clone(),
            ));
        }
        for sp in resp.storage_proof {
            let mut path = Vec::with_capacity(sp.proof.len());
            for node in &sp.proof {
                let h = keccak256(node);
                nodes.entry(h).or_insert_with(|| node.clone());
                path.push(h);
            }
            slot_proofs.push(SlotProof {
                slot: sp.key.as_b256(),
                value: sp.value,
                path,
            });
        }
    }
    let (account_fields, account_proof) =
        account.ok_or_else(|| eyre!("no proof responses from {rpc_url}"))?;

    Ok(Snapshot {
        version: SNAPSHOT_VERSION.into(),
        binding: Binding {
            chain_id: profile.chain_id,
            registry: profile.registry,
        },
        anchor: Anchor {
            block_number: anchor.block_number,
            block_hash: anchor.block_hash,
            state_root: anchor.state_root,
        },
        item_count: count,
        rows,
        proofs: Proofs {
            account_fields,
            account: account_proof,
            nodes,
            slots: slot_proofs,
        },
    })
}

/// TEST-ONLY companion to `Profile::test_headerless_state_root`: derive the
/// state root from a probe proof's root node at the given block
/// (`keccak256(accountProof[0])`), for fork/dev chains whose headers carry no
/// state root. The returned root is PROVIDER-derived, not header-authenticated.
pub async fn probe_state_root(rpc_url: &str, profile: &Profile, block: u64) -> Result<B256> {
    let provider = checked_provider(rpc_url, profile).await?;
    let resp = crate::anchor::with_deadline(&format!("{rpc_url} probe getProof"), async {
        provider
            .get_proof(
                profile.registry,
                vec![B256::from(U256::from(ITEM_LIST_SLOT))],
            )
            .block_id(BlockId::from(block))
            .await
            .map_err(|e| eyre!("{e}"))
    })
    .await?;
    let root_node = resp
        .account_proof
        .first()
        .ok_or_else(|| eyre!("{rpc_url}: probe proof is empty"))?;
    Ok(keccak256(root_node))
}

/// Freshly proven status of ONE item at the authenticated anchor (spec §8's
/// point check): EIP-1186 account proof (with the codehash pin) plus the status
/// slot, verified locally against the quorum anchor's state root.
pub async fn point_check(
    rpc_url: &str,
    profile: &Profile,
    anchor: &QuorumAnchor,
    item_id: B256,
) -> Result<u8> {
    let limits = Limits::default();
    let provider = checked_provider(rpc_url, profile).await?;
    let slot = item_status_slot(item_id);
    let meta_slot = B256::from(U256::from(META_EVIDENCE_UPDATES_SLOT));
    let resp = crate::anchor::with_deadline(&format!("{rpc_url} eth_getProof"), async {
        provider
            .get_proof(profile.registry, vec![slot, meta_slot])
            .block_id(BlockId::from(anchor.block_number))
            .await
            .map_err(|e| eyre!("{e}"))
    })
    .await?;
    let fields = AccountFields {
        nonce: resp.nonce,
        balance: resp.balance,
        storage_root: resp.storage_hash,
        code_hash: resp.code_hash,
    };
    let storage_root = verify_account(
        anchor.state_root,
        profile.registry,
        &fields,
        &resp.account_proof,
        profile.registry_code_hash,
        &limits,
    )?;
    // Policy immutability at the FRESH anchor too (spec §8): the counter could
    // have moved between the snapshot's anchor and now.
    let mp = resp
        .storage_proof
        .iter()
        .find(|p| p.key.as_b256() == meta_slot)
        .ok_or_else(|| eyre!("{rpc_url}: no storage proof for metaEvidenceUpdates"))?;
    verify_slot(storage_root, meta_slot, mp.value, &mp.proof, &limits)?;
    if mp.value != U256::ZERO {
        bail!(
            "policy immutability violated at block {}: metaEvidenceUpdates = {} (fail closed)",
            anchor.block_number,
            mp.value
        );
    }
    let sp = resp
        .storage_proof
        .iter()
        .find(|p| p.key.as_b256() == slot)
        .ok_or_else(|| eyre!("{rpc_url}: no storage proof for the status slot"))?;
    verify_slot(storage_root, slot, sp.value, &sp.proof, &limits)?;
    if sp.value > U256::from(3u64) {
        bail!("undecodable status {} (failing closed)", sp.value);
    }
    Ok(sp.value.to::<u8>())
}
