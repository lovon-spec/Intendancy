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
/// `metaEvidenceUpdates` counter — the policy VERSION (owner decision
/// 2026-09-06, superseding the immutable-policy rule): a Registered verdict is
/// only meaningful under a policy the profile has vouched for, so every
/// verification proves this counter is an ACCEPTED version (0, the deployment
/// policy, or one the profile lists).
/// PROVISIONAL pending acceptance of its evidence record
/// (`docs/spikes/meta-evidence-slot-evidence.md`): pinned by an asserting
/// probe (getter and slot moving 0→1→2 in lockstep, all other SAMPLED slots
/// 0..16 unchanged) plus the verified-source pin of the load-bearing
/// no-reset property, for the same codehash.
pub const META_EVIDENCE_UPDATES_SLOT: u64 = 9;
/// `arbitrator` (slot 0) and `arbitratorExtraData` (slot 1, a Solidity `bytes`):
/// the court a verdict came from and its parameters. Owner decision
/// (2026-09-04): both are MANDATORY profile pins and every verification proves
/// them — a governor may switch the arbitrator or the court, and to a consumer
/// that is a policy-grade change, accepted only through a new signed profile,
/// never silently. PROVISIONAL like slot 9, pinned by an asserting fork probe
/// (`cli/tools/probe-arbitrator-slots.sh`, `docs/spikes/arbitrator-slot-evidence.md`):
/// a governor `changeArbitrator` moves exactly these words, in both the short
/// (< 32 bytes, inline) and long (≥ 32 bytes, at `keccak256(1) + i`) forms.
pub const ARBITRATOR_SLOT: u64 = 0;
pub const ARBITRATOR_EXTRA_DATA_SLOT: u64 = 1;
/// `governor` (slot 3): who may queue every other change — the timelock in
/// front of the Safe (RFC 0001 §9). MANDATORY profile pin, proven at every
/// anchor (owner decision 2026-09-06). PROVISIONAL like slots 0 and 1, pinned
/// by the same fork probe (assertion F: slot 3 holds `governor()` and a
/// governor `changeGovernor` moves exactly that word).
pub const GOVERNOR_SLOT: u64 = 3;
/// Bound on the pinned/proven extra data (Kleros encodes court + jurors in 64
/// bytes; nothing legitimate approaches this).
pub const MAX_ARBITRATOR_EXTRA_DATA_BYTES: usize = 4096;

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
    /// The pinned arbitrator and its extra data (spec §3; slots 0 and 1).
    pub arbitrator: Address,
    pub arbitrator_extra_data: Bytes,
    /// The pinned governor (spec §3; slot 3).
    pub governor: Address,
    /// Accepted `metaEvidenceUpdates` values beyond 0 (spec §3 `policyVersions`).
    pub accepted_policy_updates: Vec<u64>,
    pub anchor_block: u64,
    pub anchor_block_hash: B256,
    pub anchor_state_root: B256,
}

/// Whether a proven `metaEvidenceUpdates` value is a policy version the profile
/// accepts: 0 (the deployment policy) or one of the listed updates (spec §3).
pub fn policy_version_accepted(updates: U256, accepted: &[u64]) -> bool {
    updates == U256::ZERO || accepted.iter().any(|u| U256::from(*u) == updates)
}

/// "0, 1, 2" — the accepted versions, for error messages.
pub fn accepted_versions_label(accepted: &[u64]) -> String {
    let mut v: Vec<String> = vec!["0".into()];
    v.extend(accepted.iter().map(|u| u.to_string()));
    v.join(", ")
}

/// Storage slots a Solidity `bytes` of `len` bytes at `ARBITRATOR_EXTRA_DATA_SLOT`
/// occupies: the main word always; `ceil(len / 32)` data words at
/// `keccak256(slot) + i` when `len >= 32` (long form). For generation and for
/// the fresh point check, where the pinned length is known.
pub fn extra_data_slots_for_len(len: usize) -> Vec<B256> {
    let main = B256::from(U256::from(ARBITRATOR_EXTRA_DATA_SLOT));
    let mut slots = vec![main];
    if len >= 32 {
        let base = U256::from_be_bytes(keccak256(main).0);
        for i in 0..len.div_ceil(32) {
            slots.push(B256::from(base + U256::from(i as u64)));
        }
    }
    slots
}

/// The byte length a proven `bytes` main word declares, and whether it is the
/// long form; fails closed on a non-canonical word or an absurd length.
pub fn bytes_storage_len(main: U256) -> Result<(usize, bool)> {
    let word = B256::from(main);
    let low = word[31];
    if low & 1 == 1 {
        let len = (main - U256::from(1u64)) / U256::from(2u64);
        if len > U256::from(MAX_ARBITRATOR_EXTRA_DATA_BYTES as u64) {
            bail!("arbitratorExtraData declares {len} bytes, above the {MAX_ARBITRATOR_EXTRA_DATA_BYTES} byte bound (fail closed)");
        }
        let len = len.to::<usize>();
        if len < 32 {
            bail!("arbitratorExtraData main word is a long form of {len} bytes, which Solidity never writes (fail closed)");
        }
        Ok((len, true))
    } else {
        let len = (low / 2) as usize;
        if len >= 32 {
            bail!("arbitratorExtraData main word is a short form of {len} bytes, which Solidity never writes (fail closed)");
        }
        Ok((len, false))
    }
}

/// Decode a Solidity `bytes` from its proven main word and, for the long form,
/// its data words in order; the words beyond the declared length must be zero.
pub fn decode_bytes_storage(main: U256, data_words: &[U256]) -> Result<Vec<u8>> {
    let (len, long) = bytes_storage_len(main)?;
    let mut out = Vec::with_capacity(len);
    if !long {
        if !data_words.is_empty() {
            bail!("short-form arbitratorExtraData with data words");
        }
        let word = B256::from(main);
        out.extend_from_slice(&word[..len]);
        // canonical short form: bytes after the data up to the length byte are zero
        if word[len..31].iter().any(|b| *b != 0) {
            bail!("non-canonical short-form arbitratorExtraData word (fail closed)");
        }
        return Ok(out);
    }
    let words = len.div_ceil(32);
    if data_words.len() != words {
        bail!(
            "arbitratorExtraData needs {words} data words, {} proven",
            data_words.len()
        );
    }
    for (i, w) in data_words.iter().enumerate() {
        let bytes = B256::from(*w);
        let take = (len - i * 32).min(32);
        out.extend_from_slice(&bytes[..take]);
        if bytes[take..].iter().any(|b| *b != 0) {
            bail!("non-canonical arbitratorExtraData data word {i} (fail closed)");
        }
    }
    Ok(out)
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

/// Hard ceiling on account-proof nodes accepted at DESERIALIZATION time
/// (PR #3 re-review): a `"0x"` element costs ~4 JSON bytes but tens of heap
/// bytes, so an in-cap document could otherwise allocate gigabytes before
/// `verify`'s count checks run. 128 exceeds every honest path (secure-trie
/// paths are ≤ 65 nodes; `Limits::max_path_nodes` defaults to 66) while
/// capping the amplification at the wire boundary.
pub const MAX_ACCOUNT_PROOF_NODES_WIRE: usize = 128;

fn bounded_account_proof<'de, D>(de: D) -> Result<Vec<Bytes>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct V;
    impl<'de> serde::de::Visitor<'de> for V {
        type Value = Vec<Bytes>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(
                f,
                "an account proof of at most {MAX_ACCOUNT_PROOF_NODES_WIRE} nodes"
            )
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Self::Value, A::Error> {
            let mut out = Vec::new();
            while let Some(node) = seq.next_element::<Bytes>()? {
                if out.len() >= MAX_ACCOUNT_PROOF_NODES_WIRE {
                    return Err(serde::de::Error::custom(format!(
                        "account proof exceeds {MAX_ACCOUNT_PROOF_NODES_WIRE} nodes at the \
                         wire boundary"
                    )));
                }
                out.push(node);
            }
            Ok(out)
        }
    }
    de.deserialize_seq(V)
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Proofs {
    pub account_fields: AccountFields,
    /// Bounded at deserialization — see `MAX_ACCOUNT_PROOF_NODES_WIRE`.
    #[serde(deserialize_with = "bounded_account_proof")]
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
    // 2N + 2 (length, policy counter) + the arbitrator slot + the extra-data
    // words the PINNED length occupies (spec §6 step 3c) + the governor slot
    // (step 3d).
    let arbitrator_slots =
        1 + extra_data_slots_for_len(profile.arbitrator_extra_data.len()).len() as u64;
    let max_slots = limits
        .max_items
        .saturating_mul(2)
        .saturating_add(2)
        .saturating_add(arbitrator_slots)
        .saturating_add(1);
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

    // Step 3b: policy version (spec §6; owner decision 2026-09-06). The
    // registry's metaEvidenceUpdates counter must be proven to be a version the
    // profile accepts: 0, the deployment policy, or one the signed profile lists
    // with its announced references. A governor's policy change, queued through
    // the timelock, is accepted only through a new signed profile, never
    // silently.
    let meta_slot = B256::from(U256::from(META_EVIDENCE_UPDATES_SLOT));
    let updates = check_slot(meta_slot, None)?;
    if !policy_version_accepted(updates, &profile.accepted_policy_updates) {
        bail!(
            "policy version {updates} is not one the profile accepts (accepted: {}) — \
             the governor changed the policy; a new signed profile release must name \
             the version (fail closed)",
            accepted_versions_label(&profile.accepted_policy_updates)
        );
    }
    // Step 3d: governor identity (spec §6; owner decision 2026-09-06). Who may
    // queue every other change is part of the trust context.
    let gov_word = B256::from(check_slot(B256::from(U256::from(GOVERNOR_SLOT)), None)?);
    if gov_word[..12].iter().any(|b| *b != 0) {
        bail!("governor slot holds a non-address word {gov_word} (fail closed)");
    }
    let governor = Address::from_word(gov_word);
    if governor != profile.governor {
        bail!(
            "governor pin violated: the registry's governor is {governor}, the profile pins {} — \
             a changed governor needs a new signed profile release (fail closed)",
            profile.governor
        );
    }

    // Step 3c: arbitrator identity (spec §6; owner decision 2026-09-04). The
    // court a verdict came from, and its parameters, are consumer-visible
    // policy: a governor may switch them, and a switch is accepted only through
    // a new signed profile, never silently. Both are proven at every anchor.
    let arb_word = check_slot(B256::from(U256::from(ARBITRATOR_SLOT)), None)?;
    let arb_bytes = B256::from(arb_word);
    if arb_bytes[..12].iter().any(|b| *b != 0) {
        bail!("arbitrator slot holds a non-address word {arb_word} (fail closed)");
    }
    let arbitrator = Address::from_word(arb_bytes);
    if arbitrator != profile.arbitrator {
        bail!(
            "arbitrator pin violated: the registry's arbitrator is {arbitrator}, the profile pins {} — \
             a changed arbitrator needs a new signed profile release (fail closed)",
            profile.arbitrator
        );
    }
    let main = check_slot(B256::from(U256::from(ARBITRATOR_EXTRA_DATA_SLOT)), None)?;
    let (proven_len, long) = bytes_storage_len(main)?;
    if proven_len != profile.arbitrator_extra_data.len() {
        bail!(
            "arbitrator extra data pin violated: the registry's extra data is {proven_len} bytes, the profile pins {} bytes — \
             a changed court or juror count needs a new signed profile release (fail closed)",
            profile.arbitrator_extra_data.len()
        );
    }
    let mut data_words = Vec::new();
    if long {
        for slot in extra_data_slots_for_len(proven_len).into_iter().skip(1) {
            data_words.push(check_slot(slot, None)?);
        }
    }
    let observed = decode_bytes_storage(main, &data_words)?;
    if observed.as_slice() != profile.arbitrator_extra_data.as_ref() {
        bail!(
            "arbitrator extra data pin violated: the registry's extra data is 0x{} , the profile pins 0x{} — \
             a changed court or juror count needs a new signed profile release (fail closed)",
            alloy::primitives::hex::encode(&observed),
            alloy::primitives::hex::encode(&profile.arbitrator_extra_data)
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
