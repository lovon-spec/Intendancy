//! The provisional snapshot envelope (verified-snapshot-spec §4) and its verification
//! (§6 steps 2–6), with the same `alloy-trie` MPT primitive Gate 1 uses. The anchor
//! stateRoot is a TRUSTED INPUT here — anchor derivation is Gate 1's scope.

use std::collections::BTreeMap;

use alloy::consensus::TrieAccount;
use alloy::primitives::{keccak256, Address, Bytes, B256, U256};
use alloy::rlp;
use alloy_trie::{proof::verify_proof, Nibbles};
use eyre::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::schema::Descriptor;

/// Classic GTCR storage layout (pinned; same constants Gate 1 verified empirically).
pub const ITEM_LIST_SLOT: u64 = 13;
pub const ITEMS_MAPPING_SLOT: u64 = 14;

/// Envelope version this bench emits and pins.
pub const SNAPSHOT_VERSION: &str = "bench-0.1";

/// Resource bounds enforced on untrusted snapshot input. Every bound is exercised by
/// a limit test (`tests/limits.rs`); the FINAL values are CHANGES-REQUIRED and frozen
/// only together with the binary transport framing (results §4) — these are working
/// values, not spec numbers.
#[derive(Debug, Clone)]
pub struct Limits {
    /// Cap on the compressed input file (gzip transport form).
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
    /// Cap per proof path (account or storage). Secure-trie keys are 64 nibbles, so
    /// an honest path can never exceed 65 nodes.
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

/// What the VERIFIER pins out of band (registry config / lockfile / Gate 1 anchor) —
/// never taken from the snapshot itself. `verify` accepts a snapshot only if its
/// binding, anchor, and PROVEN account codehash all match this profile exactly; a
/// provider proving some other account (e.g. a funded EOA with an honestly empty
/// storage trie) under the authentic root is rejected here.
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
    /// The finalized block's hash — the spec anchors to block/hash, not only
    /// height + stateRoot, so the snapshot carries and `verify` pins all three.
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

/// Account fields as claimed by the provider. They are TRUSTLESS despite being
/// provider-supplied: `verify` proves exactly these fields against the state root via
/// the account proof — any lie fails the MPT check.
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
        // A zero value has two provable forms, both sound against a committed root:
        // absence (the only form a REAL Ethereum trie produces — SSTORE 0 deletes),
        // and an explicit RLP(0x80) leaf, which anvil's fork-mode synthetic trie
        // emits for slots it has locally zeroed. Accept either; a nonzero slot can
        // prove neither.
        return verify_proof(root, key, None, proof)
            .or_else(|_| verify_proof(root, key, Some(vec![rlp::EMPTY_STRING_CODE]), proof))
            .map_err(|e| eyre::eyre!("{e}"));
    }
    verify_proof(root, key, Some(encoded), proof).map_err(|e| eyre::eyre!("{e}"))
}

/// Full verification per spec §6 steps 2–6 against a pinned verifier profile whose
/// anchor state root is TRUSTED (anchor derivation is Gate 1's scope).
pub fn verify(
    snapshot: &Snapshot,
    profile: &VerifierProfile,
    limits: &Limits,
) -> Result<VerifyStats> {
    // Step 1 of the profile binding: every identity field must match the pin exactly
    // BEFORE any proof is consulted. Nothing in the snapshot names its own trust.
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

    // Structural resource bounds on the untrusted envelope.
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
    if snapshot.proofs.account.len() > limits.max_path_nodes {
        bail!(
            "account proof path length {} exceeds bound {}",
            snapshot.proofs.account.len(),
            limits.max_path_nodes
        );
    }
    for node in &snapshot.proofs.account {
        if node.len() > limits.max_node_bytes {
            bail!("account proof node exceeds {} bytes", limits.max_node_bytes);
        }
    }
    let max_slots = limits.max_items.saturating_mul(2).saturating_add(1);
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

    // Step 2: account proof → storageRoot. The claimed fields are proven wholesale:
    // if any field (including storageRoot) were false, this MPT check fails.
    let af = &snapshot.proofs.account_fields;
    let account = TrieAccount {
        nonce: af.nonce,
        balance: af.balance,
        storage_root: af.storage_root,
        code_hash: af.code_hash,
    };
    verify_mpt(
        trusted_state_root,
        snapshot.binding.registry,
        account,
        &snapshot.proofs.account,
    )
    .map_err(|e| eyre::eyre!("account proof: {e}"))?;
    // Step 2b of the profile binding: the PROVEN runtime codeHash must equal the pin.
    // This is what makes "prove a different account under the authentic root" fail
    // even if the verifier's registry pin were somehow wrong: an EOA proves
    // keccak256(""), a different contract proves its own codehash.
    if af.code_hash != profile.registry_code_hash {
        bail!(
            "proven codeHash {} != pinned registry codeHash {}",
            af.code_hash,
            profile.registry_code_hash
        );
    }
    let storage_root = af.storage_root;

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
        // Descriptor must decode as the six-column schema (well-formedness).
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
/// file is. The LOGICAL RETAINED LENGTH is bounded per BUFFER, not by a single
/// cap: the on-disk buffer holds ≤ applicable cap + 1 bytes, and for gzip input
/// the decode in `read_snapshot_bounded` then materializes a SECOND, separately
/// capped buffer of ≤ decoded cap + 1 bytes — total retained bytes are bounded
/// by the sum of the two caps (heap CAPACITY may exceed the logical length by
/// the allocator's growth overhead — `Vec` over-allocates; the bound is on
/// bytes retained, not the allocator's rounding). All cap arithmetic is saturating, so degenerate injected limits (zero,
/// tiny, `u64::MAX`) reject or pass cleanly instead of under-/overflowing.
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
    // The sniffed bytes may already exceed a degenerate cap (0 or 1) — reject now;
    // this also makes the subtraction below safe.
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
/// JSON envelope or its gzip transport form (magic `1f 8b`); the compressed cap
/// applies to the input and the decoded cap bounds decompression (gzip-bomb ceiling)
/// as well as raw input. Returns the parsed snapshot plus the decoded byte count.
/// File-boundary callers must use `read_snapshot_file_bounded`, which enforces the
/// caps BEFORE materializing the input.
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
