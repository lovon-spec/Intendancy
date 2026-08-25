//! The snapshot envelope (spec §4, debug/JSON encoding) and its verification per
//! spec §6 steps 0–7 against a pinned verifier profile. Ported from the Gate 2
//! reference implementation, including the reviewer-driven hardening: profile
//! binding before any proof, bounded file-boundary reads with saturating cap
//! arithmetic, per-branch resource limits, both provable zero-slot forms, and the
//! proven-codehash equality that defeats wrong-account proofs.
//!
//! Anchor TRUST is the caller's job (spec §7 modes; see `anchor.rs`): `verify`
//! treats the profile's anchor triple as authenticated input.

use std::collections::BTreeMap;

use alloy::consensus::TrieAccount;
use alloy::primitives::{keccak256, Address, Bytes, B256, U256};
use alloy::rlp;
use alloy_trie::{proof::verify_proof, Nibbles};
use eyre::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::schema::Descriptor;

/// Classic GTCR storage layout — spec §5, FROZEN for runtime codehash
/// `0x5a6cf793…5d7d` (slots 13/14; slot 9 below is the one PROVISIONAL
/// exception); void against any other bytecode.
pub const ITEM_LIST_SLOT: u64 = 13;
pub const ITEMS_MAPPING_SLOT: u64 = 14;
/// `metaEvidenceUpdates` counter — the policy-immutability invariant (owner
/// decision, spec §11): a Registered verdict is only meaningful under the
/// deployment policy, so every verification proves this counter is STILL ZERO.
/// PROVISIONAL pending acceptance of its evidence record
/// (`docs/spikes/meta-evidence-slot-evidence.md`): pinned by an asserting
/// probe (getter and slot moving 0→1→2 in lockstep, all other SAMPLED slots
/// 0..16 unchanged) plus the verified-source pin of the load-bearing
/// no-reset property, for the same codehash.
pub const META_EVIDENCE_UPDATES_SLOT: u64 = 9;

/// Envelope revision this implementation emits and accepts (spec §4).
pub const SNAPSHOT_VERSION: &str = "0.2";

/// Resource bounds on untrusted snapshot input (spec §4.1: mechanisms normative,
/// values provisional pending the binary framing decision).
#[derive(Debug, Clone)]
pub struct Limits {
    /// Cap on the compressed input (gzip transport form).
    pub max_compressed_bytes: u64,
    /// Cap on the decoded/raw envelope bytes (also the gzip-bomb ceiling).
    pub max_decoded_bytes: u64,
    /// Cap per MPT node — applies to storage-trie AND account-proof nodes.
    pub max_node_bytes: usize,
    /// Cap on the number of entries in the deduplicated node store.
    pub max_nodes: u64,
    /// Cap on the aggregate byte size of the deduplicated node store.
    pub max_node_store_bytes: u64,
    /// Cap on itemCount / row count.
    pub max_items: u64,
    /// Cap per proof path (account or storage). Secure-trie keys are 64 nibbles,
    /// so an honest path can never exceed 65 nodes.
    pub max_path_nodes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_compressed_bytes: 64 * 1024 * 1024,
            max_decoded_bytes: 256 * 1024 * 1024,
            max_node_bytes: 16 * 1024,
            max_nodes: 8_000_000,
            max_node_store_bytes: 192 * 1024 * 1024,
            max_items: 1_000_000,
            max_path_nodes: 66,
        }
    }
}

/// What the VERIFIER pins out of band — never taken from the snapshot itself
/// (spec §3 + §6 step 0). The anchor triple comes from an authenticated header
/// (spec §7); registry identity and codehash come from the local profile.
#[derive(Debug, Clone)]
pub struct VerifierProfile {
    pub version: String,
    pub chain_id: u64,
    pub registry: Address,
    pub registry_code_hash: B256,
    pub anchor_block: u64,
    pub anchor_block_hash: B256,
    pub anchor_state_root: B256,
}

pub fn item_list_slot(index: u64) -> B256 {
    let base = keccak256(B256::from(U256::from(ITEM_LIST_SLOT)));
    B256::from(U256::from_be_bytes(base.0) + U256::from(index))
}

pub fn item_status_slot(item_id: B256) -> B256 {
    let mut buf = [0u8; 64];
    buf[..32].copy_from_slice(item_id.as_slice());
    buf[32..].copy_from_slice(B256::from(U256::from(ITEMS_MAPPING_SLOT)).as_slice());
    B256::from(U256::from_be_bytes(keccak256(buf).0) + U256::from(1u64))
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Binding {
    pub chain_id: u64,
    pub registry: Address,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Anchor {
    pub block_number: u64,
    /// The finalized block's hash — the spec anchors to the number/hash/stateRoot
    /// triple; §6 step 1 compares all three.
    pub block_hash: B256,
    pub state_root: B256,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub index: u64,
    pub item_id: B256,
    pub status: u8,
    pub descriptor: Bytes,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SlotProof {
    pub slot: B256,
    pub value: U256,
    /// Root-to-leaf path as keys into `nodes`.
    pub path: Vec<B256>,
}

/// Account fields as claimed by the provider — TRUSTLESS despite being
/// provider-supplied: `verify` proves exactly these fields against the state root
/// via the account proof; any lie fails the MPT check.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AccountFields {
    pub nonce: u64,
    pub balance: U256,
    pub storage_root: B256,
    pub code_hash: B256,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Proofs {
    pub account_fields: AccountFields,
    pub account: Vec<Bytes>,
    /// Deduplicated Merkle-Patricia nodes keyed by keccak256(node).
    pub nodes: BTreeMap<B256, Bytes>,
    pub slots: Vec<SlotProof>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub version: String,
    pub binding: Binding,
    pub anchor: Anchor,
    pub item_count: u64,
    pub rows: Vec<Row>,
    pub proofs: Proofs,
}

/// Statistics of a completed full verification.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyStats {
    pub items: u64,
    pub slot_proofs_checked: u64,
    pub unique_nodes: u64,
    pub storage_root: B256,
}

fn verify_mpt<K: AsRef<[u8]>, V: rlp::Encodable>(
    root: B256,
    raw_key: K,
    raw_value: V,
    proof: &[Bytes],
) -> Result<()> {
    let key = Nibbles::unpack(keccak256(raw_key));
    let encoded = rlp::encode(raw_value);
    if encoded.as_slice() == [rlp::EMPTY_STRING_CODE] {
        // Spec §5: a zero value has two provable forms, both sound against a
        // committed root — absence (the only form a REAL Ethereum trie produces;
        // SSTORE 0 deletes) and an explicit RLP(0x80) leaf (synthetic tries, e.g.
        // anvil fork mode in the test environment). Accept either; a nonzero slot
        // can prove neither.
        return verify_proof(root, key, None, proof)
            .or_else(|_| verify_proof(root, key, Some(vec![rlp::EMPTY_STRING_CODE]), proof))
            .map_err(|e| eyre::eyre!("{e}"));
    }
    verify_proof(root, key, Some(encoded), proof).map_err(|e| eyre::eyre!("{e}"))
}

/// Verify an EIP-1186 account proof against a trusted state root, pin the proven
/// codehash, and return the proven storage root. The building block shared by full
/// snapshot verification and the fresh point check (spec §8).
pub fn verify_account(
    state_root: B256,
    registry: Address,
    fields: &AccountFields,
    account_proof: &[Bytes],
    pinned_code_hash: B256,
    limits: &Limits,
) -> Result<B256> {
    if account_proof.len() > limits.max_path_nodes {
        bail!(
            "account proof path length {} exceeds bound {}",
            account_proof.len(),
            limits.max_path_nodes
        );
    }
    for node in account_proof {
        if node.len() > limits.max_node_bytes {
            bail!("account proof node exceeds {} bytes", limits.max_node_bytes);
        }
    }
    let account = TrieAccount {
        nonce: fields.nonce,
        balance: fields.balance,
        storage_root: fields.storage_root,
        code_hash: fields.code_hash,
    };
    verify_mpt(state_root, registry, account, account_proof)
        .map_err(|e| eyre::eyre!("account proof: {e}"))?;
    // The wrong-account defense (spec §3/§10): an EOA proves keccak256(""), a
    // different contract proves its own codehash — neither matches the pin.
    if fields.code_hash != pinned_code_hash {
        bail!(
            "proven codeHash {} != pinned registry codeHash {}",
            fields.code_hash,
            pinned_code_hash
        );
    }
    Ok(fields.storage_root)
}

/// Verify one storage slot against a proven storage root (point-check building
/// block). `proof` is the raw node list root-to-leaf.
pub fn verify_slot(
    storage_root: B256,
    slot: B256,
    value: U256,
    proof: &[Bytes],
    limits: &Limits,
) -> Result<()> {
    if proof.len() > limits.max_path_nodes {
        bail!(
            "slot proof path length {} exceeds bound {}",
            proof.len(),
            limits.max_path_nodes
        );
    }
    for node in proof {
        if node.len() > limits.max_node_bytes {
            bail!("slot proof node exceeds {} bytes", limits.max_node_bytes);
        }
    }
    verify_mpt(storage_root, slot, value, proof).map_err(|e| eyre::eyre!("slot {slot}: {e}"))
}

/// Full verification per spec §6 steps 0–6 against the pinned profile (whose
/// anchor triple the caller has authenticated per §7).
pub fn verify(
    snapshot: &Snapshot,
    profile: &VerifierProfile,
    limits: &Limits,
) -> Result<VerifyStats> {
    // Step 0: identity binding BEFORE any proof. Nothing in the snapshot names
    // its own trust.
    if snapshot.version != profile.version {
        bail!(
            "snapshot version {:?} != pinned version {:?}",
            snapshot.version,
            profile.version
        );
    }
    if snapshot.binding.chain_id != profile.chain_id {
        bail!(
            "snapshot chainId {} != pinned chainId {}",
            snapshot.binding.chain_id,
            profile.chain_id
        );
    }
    if snapshot.binding.registry != profile.registry {
        bail!(
            "snapshot registry {} != pinned registry {}",
            snapshot.binding.registry,
            profile.registry
        );
    }
    // Step 1 (anchor triple vs the authenticated header the profile carries).
    if snapshot.anchor.block_number != profile.anchor_block {
        bail!(
            "snapshot anchor block {} != pinned anchor block {}",
            snapshot.anchor.block_number,
            profile.anchor_block
        );
    }
    if snapshot.anchor.block_hash != profile.anchor_block_hash {
        bail!(
            "snapshot anchor block hash {} != pinned anchor block hash {}",
            snapshot.anchor.block_hash,
            profile.anchor_block_hash
        );
    }
    if snapshot.anchor.state_root != profile.anchor_state_root {
        bail!(
            "snapshot anchor stateRoot {} does not match the trusted root {}",
            snapshot.anchor.state_root,
            profile.anchor_state_root
        );
    }
    let trusted_state_root = profile.anchor_state_root;

    // Structural resource bounds on the untrusted envelope (§4.1).
    if snapshot.item_count > limits.max_items {
        bail!(
            "itemCount {} exceeds bound {}",
            snapshot.item_count,
            limits.max_items
        );
    }
    if snapshot.rows.len() as u64 > limits.max_items {
        bail!(
            "row count {} exceeds bound {}",
            snapshot.rows.len(),
            limits.max_items
        );
    }
    if snapshot.proofs.nodes.len() as u64 > limits.max_nodes {
        bail!(
            "node count {} exceeds bound {}",
            snapshot.proofs.nodes.len(),
            limits.max_nodes
        );
    }
    // Length slot + policy-counter slot + per-item (list + status) slots.
    let max_slots = limits.max_items.saturating_mul(2).saturating_add(2);
    if snapshot.proofs.slots.len() as u64 > max_slots {
        bail!(
            "slot proof count {} exceeds bound {max_slots}",
            snapshot.proofs.slots.len()
        );
    }
    let mut store_bytes = 0u64;
    for (hash, node) in &snapshot.proofs.nodes {
        if node.len() > limits.max_node_bytes {
            bail!("node {hash} exceeds {} bytes", limits.max_node_bytes);
        }
        store_bytes += node.len() as u64;
        if store_bytes > limits.max_node_store_bytes {
            bail!(
                "node store exceeds aggregate bound {} bytes",
                limits.max_node_store_bytes
            );
        }
        if keccak256(node) != *hash {
            bail!("node store key {hash} does not hash its contents (corrupt store)");
        }
    }
    for sp in &snapshot.proofs.slots {
        if sp.path.len() > limits.max_path_nodes {
            bail!(
                "slot {} proof path length {} exceeds bound {}",
                sp.slot,
                sp.path.len(),
                limits.max_path_nodes
            );
        }
    }

    // Step 2: account proof → proven storageRoot + codehash pin.
    let storage_root = verify_account(
        trusted_state_root,
        snapshot.binding.registry,
        &snapshot.proofs.account_fields,
        &snapshot.proofs.account,
        profile.registry_code_hash,
        limits,
    )?;

    // Index slot proofs by slot key for the sweep.
    let mut by_slot: BTreeMap<B256, &SlotProof> = BTreeMap::new();
    for sp in &snapshot.proofs.slots {
        if by_slot.insert(sp.slot, sp).is_some() {
            bail!("duplicate slot proof for {}", sp.slot);
        }
    }
    let mut checked = 0u64;
    let mut check_slot = |slot: B256, want: Option<U256>| -> Result<U256> {
        let sp = by_slot
            .get(&slot)
            .ok_or_else(|| eyre::eyre!("missing proof for slot {slot}"))?;
        let mut path_nodes: Vec<Bytes> = Vec::with_capacity(sp.path.len());
        for h in &sp.path {
            let node = snapshot
                .proofs
                .nodes
                .get(h)
                .ok_or_else(|| eyre::eyre!("slot {slot} references missing node {h}"))?;
            path_nodes.push(node.clone());
        }
        verify_mpt(storage_root, slot, sp.value, &path_nodes)
            .map_err(|e| eyre::eyre!("slot {slot}: {e}"))?;
        if let Some(w) = want {
            if sp.value != w {
                bail!("slot {slot} proven value {} != expected {w}", sp.value);
            }
        }
        checked += 1;
        Ok(sp.value)
    };

    // Step 3: proven length equals itemCount equals rows.
    let len_slot = B256::from(U256::from(ITEM_LIST_SLOT));
    let proven_len = check_slot(len_slot, Some(U256::from(snapshot.item_count)))?;
    if proven_len != U256::from(snapshot.rows.len() as u64) {
        bail!(
            "rows {} != proven itemCount {proven_len}",
            snapshot.rows.len()
        );
    }

    // Step 3b: policy immutability (spec §6; owner decision). The registry's
    // metaEvidenceUpdates counter must be proven ZERO — the deployment
    // MetaEvidence/policy is then the only one ever declared, and the profile's
    // pinned policy CIDs are what every verdict was judged under. Any update
    // fails closed: changing policy means deploying a new registry.
    let meta_slot = B256::from(U256::from(META_EVIDENCE_UPDATES_SLOT));
    let updates = check_slot(meta_slot, None)?;
    if updates != U256::ZERO {
        bail!(
            "policy immutability violated: metaEvidenceUpdates = {updates} — the \
             registry's policy was changed after deployment (fail closed; V1 pins \
             one immutable policy per registry)"
        );
    }

    // Steps 4–6: contiguous enumeration, descriptor re-hash, status sweep.
    for (i, row) in snapshot.rows.iter().enumerate() {
        if row.index != i as u64 {
            bail!("row {i} carries index {} (must be contiguous)", row.index);
        }
        check_slot(
            item_list_slot(row.index),
            Some(U256::from_be_bytes(row.item_id.0)),
        )?;
        let derived = keccak256(&row.descriptor);
        if derived != row.item_id {
            bail!(
                "row {i}: descriptor hashes to {derived}, itemID is {}",
                row.item_id
            );
        }
        Descriptor::decode(&row.descriptor)
            .map_err(|e| eyre::eyre!("row {i}: descriptor malformed: {e}"))?;
        let status = check_slot(item_status_slot(row.item_id), None)?;
        if status > U256::from(3u64) {
            bail!("row {i}: undecodable status {status}");
        }
        if status != U256::from(row.status as u64) {
            bail!("row {i}: claimed status {} != proven {status}", row.status);
        }
    }

    Ok(VerifyStats {
        items: snapshot.item_count,
        slot_proofs_checked: checked,
        unique_nodes: snapshot.proofs.nodes.len() as u64,
        storage_root,
    })
}

/// Open and parse an untrusted snapshot FILE under the byte limits. The first two
/// bytes are sniffed for the gzip magic, then the file is streamed through
/// `Read::take` at the applicable cap (compressed cap for gzip, decoded cap for
/// raw), so at most cap + 1 bytes are ever read FROM DISK no matter how large the
/// file is. The LOGICAL RETAINED LENGTH is bounded per BUFFER: the on-disk
/// buffer holds ≤ applicable cap + 1 bytes, and for gzip input the decode then
/// materializes a SECOND, separately capped buffer of ≤ decoded cap + 1 bytes
/// (heap capacity may exceed the logical length by `Vec`'s allocator growth
/// overhead — the bound is on bytes retained, not the allocator's rounding). All cap arithmetic is saturating, so
/// degenerate configured limits reject or pass cleanly instead of
/// under-/overflowing.
pub fn read_snapshot_file_bounded(
    path: &std::path::Path,
    limits: &Limits,
) -> Result<(Snapshot, u64)> {
    use std::io::Read;
    let mut file =
        std::fs::File::open(path).map_err(|e| eyre::eyre!("opening {}: {e}", path.display()))?;
    let mut magic = [0u8; 2];
    let mut got = 0usize;
    while got < 2 {
        match file.read(&mut magic[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(e) => return Err(eyre::eyre!("reading {}: {e}", path.display())),
        }
    }
    let is_gzip = got == 2 && magic == [0x1f, 0x8b];
    let cap = if is_gzip {
        limits.max_compressed_bytes
    } else {
        limits.max_decoded_bytes
    };
    // The sniffed bytes may already exceed a degenerate cap — reject now; this
    // also makes the subtraction below safe.
    if got as u64 > cap {
        if is_gzip {
            bail!(
                "compressed snapshot file exceeds bound {} bytes",
                limits.max_compressed_bytes
            );
        }
        bail!(
            "snapshot file exceeds decoded bound {} bytes",
            limits.max_decoded_bytes
        );
    }
    let mut raw = Vec::new();
    raw.extend_from_slice(&magic[..got]);
    file.take((cap - got as u64).saturating_add(1))
        .read_to_end(&mut raw)
        .map_err(|e| eyre::eyre!("reading {}: {e}", path.display()))?;
    if raw.len() as u64 > cap {
        if is_gzip {
            bail!(
                "compressed snapshot file exceeds bound {} bytes",
                limits.max_compressed_bytes
            );
        }
        bail!(
            "snapshot file exceeds decoded bound {} bytes",
            limits.max_decoded_bytes
        );
    }
    read_snapshot_bounded(&raw, limits)
}

/// Parse an untrusted snapshot from MEMORY under the byte limits. Accepts the raw
/// JSON envelope or its gzip transport form; the decoded cap bounds decompression
/// (gzip-bomb ceiling) as well as raw input. Returns the snapshot plus the decoded
/// byte count. File-boundary callers use `read_snapshot_file_bounded`, which
/// enforces the caps BEFORE materializing the input.
pub fn read_snapshot_bounded(raw: &[u8], limits: &Limits) -> Result<(Snapshot, u64)> {
    let decoded: Vec<u8>;
    let body: &[u8] = if raw.starts_with(&[0x1f, 0x8b]) {
        if raw.len() as u64 > limits.max_compressed_bytes {
            bail!(
                "compressed snapshot {} bytes exceeds bound {}",
                raw.len(),
                limits.max_compressed_bytes
            );
        }
        use std::io::Read;
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(raw)
            .take(limits.max_decoded_bytes.saturating_add(1))
            .read_to_end(&mut out)
            .map_err(|e| eyre::eyre!("gzip decode: {e}"))?;
        if out.len() as u64 > limits.max_decoded_bytes {
            bail!(
                "decoded snapshot exceeds bound {} bytes",
                limits.max_decoded_bytes
            );
        }
        decoded = out;
        &decoded
    } else {
        if raw.len() as u64 > limits.max_decoded_bytes {
            bail!(
                "snapshot {} bytes exceeds decoded bound {}",
                raw.len(),
                limits.max_decoded_bytes
            );
        }
        raw
    };
    let snapshot: Snapshot =
        serde_json::from_slice(body).map_err(|e| eyre::eyre!("snapshot parse: {e}"))?;
    Ok((snapshot, body.len() as u64))
}
