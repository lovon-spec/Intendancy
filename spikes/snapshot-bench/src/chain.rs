//! Anvil-fork provider side: deploy a fresh Classic GTCR through the REAL factory,
//! seed synthetic V1-schema entries, and harvest `eth_getProof` into the snapshot
//! envelope. Everything here is the PROVIDER role — untrusted by the verifier.

use std::collections::BTreeMap;
use std::time::Instant;

use alloy::network::{EthereumWallet, TransactionBuilder};
use alloy::primitives::{keccak256, Address, Bytes, B256, U256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::rpc::types::{BlockId, TransactionRequest};
use alloy::signers::local::PrivateKeySigner;
use alloy::sol_types::SolCall;
use eyre::{bail, eyre, Context, Result};
use serde::{Deserialize, Serialize};

use crate::schema::synthetic;
use crate::snapshot::{
    item_list_slot, item_status_slot, AccountFields, Anchor, Binding, Proofs, Row, SlotProof,
    Snapshot, ITEM_LIST_SLOT,
};

/// Official GTCRFactory on Gnosis (the fork serves its real bytecode).
pub const GTCR_FACTORY: Address =
    alloy::primitives::address!("794Cee5a6e1501b633eC13b8c1e327d9860FE039");
/// xKlerosLiquid on Gnosis — used only for `arbitrationCost` during seeding.
pub const XKLEROS_LIQUID: Address =
    alloy::primitives::address!("9C1dA9A04925bDfDedf0f6421bC7EEa8305F9002");
/// Anvil's default funded account #0 private key (public, test-only).
pub const ANVIL_KEY0: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

/// Generated bindings live in a module so the lint allowance actually applies —
/// the real factory ABI has 12 parameters.
#[allow(clippy::too_many_arguments)]
mod abi {
    use alloy::sol;

    sol! {
        #[sol(rpc)]
        interface IGTCRFactory {
            event NewGTCR(address indexed _address);
            function deploy(
                address _arbitrator,
                bytes _arbitratorExtraData,
                address _connectedTCR,
                string _registrationMetaEvidence,
                string _clearingMetaEvidence,
                address _governor,
                uint256 _submissionBaseDeposit,
                uint256 _removalBaseDeposit,
                uint256 _submissionChallengeBaseDeposit,
                uint256 _removalChallengeBaseDeposit,
                uint256 _challengePeriodDuration,
                uint256[3] _stakeMultipliers
            ) external;
        }

        #[sol(rpc)]
        interface IGTCR {
            function addItem(bytes _item) external payable;
            function removeItem(bytes32 _itemID, string _evidence) external payable;
            function executeRequest(bytes32 _itemID) external;
            function itemCount() external view returns (uint256);
            function getItemInfo(bytes32 _itemID)
                external view returns (bytes data, uint8 status, uint256 numberOfRequests);
        }

        #[sol(rpc)]
        interface IArbitratorView {
            function arbitrationCost(bytes _extraData) external view returns (uint256);
        }
    }
}
use abi::{IArbitratorView, IGTCRFactory, IGTCR};

/// Seed run record: everything `generate`/`verify` need, including the TRUSTED anchor
/// (trusted here because the bench owns the local chain; anchor trust is Gate 1).
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SeedManifest {
    pub rpc_url: String,
    pub chain_id: u64,
    pub registry: Address,
    pub registry_code_hash: B256,
    pub items: u64,
    pub anchor_block: u64,
    pub anchor_block_hash: B256,
    pub anchor_state_root: B256,
    pub seed_wall_secs: f64,
}

impl SeedManifest {
    /// The pinned verifier profile this bench trusts (the manifest plays the role the
    /// registry pin + Gate 1 anchor play in production).
    pub fn profile(&self) -> crate::snapshot::VerifierProfile {
        crate::snapshot::VerifierProfile {
            version: crate::snapshot::SNAPSHOT_VERSION.into(),
            chain_id: self.chain_id,
            registry: self.registry,
            registry_code_hash: self.registry_code_hash,
            anchor_block: self.anchor_block,
            anchor_block_hash: self.anchor_block_hash,
            anchor_state_root: self.anchor_state_root,
        }
    }
}

fn wallet_provider(rpc_url: &str) -> Result<impl Provider + Clone> {
    let signer: PrivateKeySigner = ANVIL_KEY0.parse()?;
    let wallet = EthereumWallet::from(signer);
    Ok(ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(rpc_url.parse()?))
}

pub async fn seed(rpc_url: &str, items: u64) -> Result<SeedManifest> {
    let provider = wallet_provider(rpc_url)?;
    let chain_id = provider.get_chain_id().await?;
    let seeder: Address = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".parse()?;

    // Arbitration cost decides the per-item deposit; fund the seeder generously.
    let extra_data: Bytes = {
        let mut b = Vec::with_capacity(64);
        b.extend_from_slice(&U256::ZERO.to_be_bytes::<32>());
        b.extend_from_slice(&U256::from(3u64).to_be_bytes::<32>());
        b.into()
    };
    let arb = IArbitratorView::new(XKLEROS_LIQUID, provider.clone());
    let cost: U256 = arb.arbitrationCost(extra_data.clone()).call().await?;
    let per_item = cost; // base deposits are zero for the bench registry
    let need = per_item * U256::from(items + 16) + U256::from(10u128.pow(20));
    let _: serde_json::Value = provider
        .raw_request("anvil_setBalance".into(), (seeder, format!("0x{need:x}")))
        .await
        .wrap_err("anvil_setBalance (is this an anvil endpoint?)")?;

    // Deploy a fresh registry through the real factory. (`deploy` is a reserved
    // method name in alloy's generated instances, so the call is encoded manually.)
    let call = IGTCRFactory::deployCall {
        _arbitrator: XKLEROS_LIQUID,
        _arbitratorExtraData: extra_data,
        _connectedTCR: Address::ZERO,
        _registrationMetaEvidence: "/ipfs/bench/registration.json".into(),
        _clearingMetaEvidence: "/ipfs/bench/clearing.json".into(),
        _governor: seeder,
        _submissionBaseDeposit: U256::ZERO,
        _removalBaseDeposit: U256::ZERO,
        _submissionChallengeBaseDeposit: U256::ZERO,
        _removalChallengeBaseDeposit: U256::ZERO,
        // 1-second challenge period (irrelevant to the bench).
        _challengePeriodDuration: U256::from(1u64),
        _stakeMultipliers: [
            U256::from(10_000u64),
            U256::from(10_000u64),
            U256::from(20_000u64),
        ],
    };
    let tx = TransactionRequest::default()
        .with_to(GTCR_FACTORY)
        .with_input(call.abi_encode());
    let receipt = provider.send_transaction(tx).await?.get_receipt().await?;
    let topic0 = keccak256(b"NewGTCR(address)");
    let registry = receipt
        .inner
        .logs()
        .iter()
        .find(|l| l.address() == GTCR_FACTORY && l.topics().first() == Some(&topic0))
        .and_then(|l| l.topics().get(1))
        .map(|t| Address::from_slice(&t.as_slice()[12..]))
        .ok_or_else(|| eyre!("NewGTCR log not found in factory deploy receipt"))?;
    let code = provider.get_code_at(registry).await?;
    let registry_code_hash = keccak256(&code);

    // Seed: sequential sends (anvil accepts each in ~ms); await the final receipt.
    let gtcr = IGTCR::new(registry, provider.clone());
    let started = Instant::now();
    let mut last = None;
    for i in 0..items {
        let descriptor = synthetic(i).encode();
        let pending = gtcr.addItem(descriptor).value(per_item).send().await?;
        last = Some(pending);
        if (i + 1) % 500 == 0 {
            // Sync point: make sure the node keeps up and nonces stay ordered.
            if let Some(p) = last.take() {
                p.get_receipt().await?;
            }
            eprintln!("seeded {}/{items}", i + 1);
        }
    }
    if let Some(p) = last {
        p.get_receipt().await?;
    }

    // Status choreography (finding: only status 2 had proof evidence). Drive the
    // first three items through the real state machine so the snapshot carries every
    // Classic status, INCLUDING an executed removal whose status slot is zero. Note
    // anvil's fork-mode trie proves that zero as an explicit RLP(0x80) LEAF (it does
    // not delete locally zeroed slots the way a real trie does); the true
    // exclusion-proof form is covered by the EOA probe's empty-trie slot:
    //   item 0: RegistrationRequested → Registered                    (status 1)
    //   item 1: … → Registered → ClearingRequested → executed removal (status 0)
    //   item 2: … → Registered → ClearingRequested                    (status 3)
    // Everything else stays RegistrationRequested (status 2). The 1-second challenge
    // period plus explicit evm time warps makes requests executable.
    if items >= 3 {
        let ids: Vec<B256> = (0..3).map(|i| synthetic(i).item_id()).collect();
        warp(&provider, 10).await?;
        for id in &ids {
            gtcr.executeRequest(*id).send().await?.get_receipt().await?;
        }
        for id in &ids[1..3] {
            gtcr.removeItem(*id, String::new())
                .value(per_item)
                .send()
                .await?
                .get_receipt()
                .await?;
        }
        warp(&provider, 10).await?;
        gtcr.executeRequest(ids[1])
            .send()
            .await?
            .get_receipt()
            .await?;
        for (i, want) in [(0u64, 1u8), (1, 0), (2, 3)] {
            let info = gtcr.getItemInfo(synthetic(i).item_id()).call().await?;
            if info.status != want {
                bail!(
                    "choreography: item {i} status {} != expected {want}",
                    info.status
                );
            }
        }
    }
    let seed_wall_secs = started.elapsed().as_secs_f64();

    let count: U256 = gtcr.itemCount().call().await?;
    if count != U256::from(items) {
        bail!("itemCount {count} != requested {items}");
    }

    let block = provider
        .get_block(BlockId::latest())
        .await?
        .ok_or_else(|| eyre!("latest block missing"))?;
    // Anvil (fork mode) does not carry a state root in block headers; its
    // `eth_getProof` nevertheless builds real proofs over its local trie. The trusted
    // anchor root for this bench is therefore the root NODE of a probe proof —
    // keccak256 of the first account-proof node. Anchor TRUST is Gate 1's scope; this
    // bench only needs a consistent root all proofs verify against.
    let probe = provider
        .get_proof(registry, vec![B256::from(U256::from(ITEM_LIST_SLOT))])
        .block_id(BlockId::latest())
        .await?;
    let root_node = probe
        .account_proof
        .first()
        .ok_or_else(|| eyre!("probe proof is empty"))?;
    let anchor_state_root = keccak256(root_node);
    Ok(SeedManifest {
        rpc_url: rpc_url.into(),
        chain_id,
        registry,
        registry_code_hash,
        items,
        anchor_block: block.header.number,
        anchor_block_hash: block.header.hash,
        anchor_state_root,
        seed_wall_secs,
    })
}

/// Advance anvil's clock and mine a block so time-gated GTCR transitions execute.
async fn warp<P: Provider>(provider: &P, secs: u64) -> Result<()> {
    let _: serde_json::Value = provider
        .raw_request("evm_increaseTime".into(), (secs,))
        .await
        .wrap_err("evm_increaseTime")?;
    let _: serde_json::Value = provider
        .raw_request("evm_mine".into(), ())
        .await
        .wrap_err("evm_mine")?;
    Ok(())
}

/// Generation metrics reported alongside the snapshot. Wall time is split by phase:
/// the sequential `getItemInfo` row sweep and the chunked `eth_getProof` harvest are
/// separate RPC workloads and must not be conflated in the results.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GenerateStats {
    pub items: u64,
    pub generate_wall_secs: f64,
    pub rows_wall_secs: f64,
    pub proofs_wall_secs: f64,
    pub item_info_rpc_calls: u64,
    pub proof_rpc_calls: u64,
    pub raw_json_bytes: u64,
    pub naive_proof_bytes: u64,
    /// Deduplicated node PAYLOAD bytes only.
    pub dedup_node_bytes: u64,
    /// 32-byte store keys for every unique node.
    pub node_keys_bytes: u64,
    /// 32-byte path references from slot proofs into the store.
    pub path_ref_bytes: u64,
    /// Full dictionary cost: payloads + keys + path references.
    pub dictionary_total_bytes: u64,
    pub unique_nodes: u64,
    pub gzip_bytes: u64,
    pub status_counts: BTreeMap<u8, u64>,
}

pub async fn generate(manifest: &SeedManifest) -> Result<(Snapshot, GenerateStats)> {
    let provider = wallet_provider(&manifest.rpc_url)?;
    let registry = manifest.registry;
    let anchor: BlockId = manifest.anchor_block.into();
    let started = Instant::now();

    // Rows: enumerate itemIDs deterministically (provider role may use any index it
    // likes; the VERIFIER only trusts proofs). Here we re-derive descriptors and read
    // status via getItemInfo.
    let gtcr = IGTCR::new(registry, provider.clone());
    let mut rows = Vec::with_capacity(manifest.items as usize);
    for i in 0..manifest.items {
        let d = synthetic(i);
        let raw = d.encode();
        let id = d.item_id();
        let info = gtcr.getItemInfo(id).block(anchor).call().await?;
        rows.push(Row {
            index: i,
            item_id: id,
            status: info.status,
            descriptor: raw,
        });
    }
    let rows_wall_secs = started.elapsed().as_secs_f64();
    let mut status_counts: BTreeMap<u8, u64> = BTreeMap::new();
    for row in &rows {
        *status_counts.entry(row.status).or_default() += 1;
    }
    let proofs_started = Instant::now();

    // Slot set: length + every itemList[i] + every status slot.
    let mut slots: Vec<B256> = Vec::with_capacity(1 + 2 * rows.len());
    slots.push(B256::from(U256::from(ITEM_LIST_SLOT)));
    for row in &rows {
        slots.push(item_list_slot(row.index));
        slots.push(item_status_slot(row.item_id));
    }

    // Harvest proofs in chunks.
    const CHUNK: usize = 250;
    let mut nodes: BTreeMap<B256, Bytes> = BTreeMap::new();
    let mut slot_proofs: Vec<SlotProof> = Vec::with_capacity(slots.len());
    let mut account: Option<(AccountFields, Vec<Bytes>)> = None;
    let mut naive_proof_bytes = 0u64;
    let mut proof_rpc_calls = 0u64;
    for chunk in slots.chunks(CHUNK) {
        let resp = provider
            .get_proof(registry, chunk.to_vec())
            .block_id(anchor)
            .await?;
        proof_rpc_calls += 1;
        if account.is_none() {
            naive_proof_bytes += resp
                .account_proof
                .iter()
                .map(|n| n.len() as u64)
                .sum::<u64>();
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
                naive_proof_bytes += node.len() as u64;
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
        account.ok_or_else(|| eyre!("no proof chunks were fetched"))?;

    let proofs_wall_secs = proofs_started.elapsed().as_secs_f64();
    let dedup_node_bytes: u64 = nodes.values().map(|n| n.len() as u64).sum();
    let unique_nodes = nodes.len() as u64;
    let node_keys_bytes = 32 * unique_nodes;
    let path_ref_bytes: u64 = 32
        * slot_proofs
            .iter()
            .map(|sp| sp.path.len() as u64)
            .sum::<u64>();
    let snapshot = Snapshot {
        version: crate::snapshot::SNAPSHOT_VERSION.into(),
        binding: Binding {
            chain_id: manifest.chain_id,
            registry,
        },
        anchor: Anchor {
            block_number: manifest.anchor_block,
            block_hash: manifest.anchor_block_hash,
            state_root: manifest.anchor_state_root,
        },
        item_count: manifest.items,
        rows,
        proofs: Proofs {
            account_fields,
            account: account_proof,
            nodes,
            slots: slot_proofs,
        },
    };

    let raw = serde_json::to_vec(&snapshot)?;
    let gzip_bytes = {
        use flate2::{write::GzEncoder, Compression};
        use std::io::Write;
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&raw)?;
        enc.finish()?.len() as u64
    };
    let stats = GenerateStats {
        items: manifest.items,
        generate_wall_secs: started.elapsed().as_secs_f64(),
        rows_wall_secs,
        proofs_wall_secs,
        item_info_rpc_calls: manifest.items,
        proof_rpc_calls,
        raw_json_bytes: raw.len() as u64,
        naive_proof_bytes,
        dedup_node_bytes,
        node_keys_bytes,
        path_ref_bytes,
        dictionary_total_bytes: dedup_node_bytes + node_keys_bytes + path_ref_bytes,
        unique_nodes,
        gzip_bytes,
        status_counts,
    };
    Ok((snapshot, stats))
}

/// Harvest the ATTACK artifact for the profile-binding tests: an honest proof that a
/// funded EOA (the seeder) has an empty `itemList` slot under the SAME anchor root —
/// packaged as a snapshot claiming an empty catalog for that address. A verifier
/// without profile binding would accept it; ours must reject it on the registry pin,
/// and on the proven-codehash pin even if the registry pin were wrong.
pub async fn eoa_probe(manifest: &SeedManifest) -> Result<Snapshot> {
    let provider = wallet_provider(&manifest.rpc_url)?;
    let seeder: Address = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".parse()?;
    let resp = provider
        .get_proof(seeder, vec![B256::from(U256::from(ITEM_LIST_SLOT))])
        .block_id(BlockId::from(manifest.anchor_block))
        .await?;
    let mut nodes: BTreeMap<B256, Bytes> = BTreeMap::new();
    let mut slots = Vec::new();
    for sp in resp.storage_proof {
        let mut path = Vec::with_capacity(sp.proof.len());
        for node in &sp.proof {
            let h = keccak256(node);
            nodes.entry(h).or_insert_with(|| node.clone());
            path.push(h);
        }
        slots.push(SlotProof {
            slot: sp.key.as_b256(),
            value: sp.value,
            path,
        });
    }
    Ok(Snapshot {
        version: crate::snapshot::SNAPSHOT_VERSION.into(),
        binding: Binding {
            chain_id: manifest.chain_id,
            registry: seeder,
        },
        anchor: Anchor {
            block_number: manifest.anchor_block,
            block_hash: manifest.anchor_block_hash,
            state_root: manifest.anchor_state_root,
        },
        item_count: 0,
        rows: Vec::new(),
        proofs: Proofs {
            account_fields: AccountFields {
                nonce: resp.nonce,
                balance: resp.balance,
                storage_root: resp.storage_hash,
                code_hash: resp.code_hash,
            },
            account: resp.account_proof,
            nodes,
            slots,
        },
    })
}
