//! Offline snapshot-fixture builder: constructs REAL Merkle-Patricia tries with
//! `alloy_trie::HashBuilder` (retaining proofs), so the verification suite runs
//! with zero network/anvil dependencies. The shape mirrors a 4-item Classic
//! registry: statuses Registered / RegistrationRequested / ClearingRequested and
//! one executed removal whose status slot is ABSENT (true exclusion proof), plus
//! a funded EOA account under the same state root for the wrong-account tests.

use std::collections::BTreeMap;

use alloy::consensus::TrieAccount;
use alloy::primitives::{keccak256, Address, Bytes, B256, U256};
use alloy_trie::proof::ProofRetainer;
use alloy_trie::{HashBuilder, Nibbles, EMPTY_ROOT_HASH};
use intend::schema::Descriptor;
use intend::snapshot::{
    extra_data_slots_for_len, item_list_slot, item_status_slot, AccountFields, Anchor, Binding,
    Proofs, Row, SlotProof, Snapshot, VerifierProfile, ARBITRATOR_EXTRA_DATA_SLOT, ARBITRATOR_SLOT,
    GOVERNOR_SLOT, ITEM_LIST_SLOT, META_EVIDENCE_UPDATES_SLOT, SNAPSHOT_VERSION,
};

pub const CHAIN_ID: u64 = 100;

pub fn registry_address() -> Address {
    Address::repeat_byte(0x11)
}

#[allow(dead_code)]
pub fn eoa_address() -> Address {
    Address::repeat_byte(0x22)
}

/// Arbitrary pinned "runtime codehash" for the fixture registry.
pub fn registry_code_hash() -> B256 {
    keccak256(b"fixture-registry-runtime-code")
}

#[allow(dead_code)]
pub fn keccak_empty() -> B256 {
    keccak256([])
}

fn sample_descriptor(i: u64) -> Descriptor {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("fixture-tree-{i}"));
    let mut cid = vec![0x01, 0x70, 0x12, 0x20];
    cid.extend_from_slice(&digest);
    Descriptor {
        name: format!("fixture-skill-{i}"),
        description: format!("Offline fixture entry {i}."),
        tree_cid: format!(
            "b{}",
            data_encoding::BASE32_NOPAD.encode(&cid).to_lowercase()
        ),
        runtimes: "generic".into(),
        origin: String::new(),
        reserved: String::new(),
    }
}

/// Build a trie over (hashed key → RLP value), retaining proofs for every target
/// key (present or absent). Returns (root, per-target proof node lists).
fn build_trie(
    entries: &BTreeMap<B256, Vec<u8>>,
    targets: &[B256],
) -> (B256, BTreeMap<B256, Vec<Bytes>>) {
    let target_nibbles: Vec<Nibbles> = targets
        .iter()
        .map(|t| Nibbles::unpack(keccak256(t)))
        .collect();
    let retainer = ProofRetainer::new(target_nibbles.clone());
    let mut builder = HashBuilder::default().with_proof_retainer(retainer);
    // Leaves must be inserted in nibble order.
    let mut sorted: Vec<(Nibbles, &Vec<u8>)> = entries
        .iter()
        .map(|(k, v)| (Nibbles::unpack(keccak256(k)), v))
        .collect();
    sorted.sort_by_key(|a| a.0);
    for (nibbles, value) in sorted {
        builder.add_leaf(nibbles, value);
    }
    let root = builder.root();
    let proof_nodes = builder.take_proof_nodes();
    let mut out = BTreeMap::new();
    for (target, nibbles) in targets.iter().zip(target_nibbles) {
        let nodes: Vec<Bytes> = proof_nodes
            .matching_nodes_sorted(&nibbles)
            .into_iter()
            .map(|(_, node)| node)
            .collect();
        out.insert(*target, nodes);
    }
    (root, out)
}

/// Address-keyed variant for the account trie.
fn build_account_trie(
    accounts: &BTreeMap<Address, TrieAccount>,
    targets: &[Address],
) -> (B256, BTreeMap<Address, Vec<Bytes>>) {
    let target_nibbles: Vec<Nibbles> = targets
        .iter()
        .map(|a| Nibbles::unpack(keccak256(a)))
        .collect();
    let retainer = ProofRetainer::new(target_nibbles.clone());
    let mut builder = HashBuilder::default().with_proof_retainer(retainer);
    let mut sorted: Vec<(Nibbles, Vec<u8>)> = accounts
        .iter()
        .map(|(a, acct)| (Nibbles::unpack(keccak256(a)), alloy::rlp::encode(acct)))
        .collect();
    sorted.sort_by_key(|a| a.0);
    for (nibbles, value) in sorted {
        builder.add_leaf(nibbles, &value);
    }
    let root = builder.root();
    let proof_nodes = builder.take_proof_nodes();
    let mut out = BTreeMap::new();
    for (target, nibbles) in targets.iter().zip(target_nibbles) {
        let nodes: Vec<Bytes> = proof_nodes
            .matching_nodes_sorted(&nibbles)
            .into_iter()
            .map(|(_, node)| node)
            .collect();
        out.insert(*target, nodes);
    }
    (root, out)
}

// Shared across test binaries; not every binary uses every field/helper.
#[allow(dead_code)]
pub struct Fixture {
    pub snapshot: Snapshot,
    pub profile: VerifierProfile,
    /// An honest empty-catalog snapshot for the EOA under the SAME root.
    pub eoa_snapshot: Snapshot,
}

pub fn build_fixture() -> Fixture {
    build_fixture_with(0, &court_extra_data(19, 3))
}

/// The pinned arbitrator of every fixture (xKlerosLiquid on Gnosis).
pub fn fixture_arbitrator() -> Address {
    "0x9C1dA9A04925bDfDedf0f6421bC7EEa8305F9002"
        .parse()
        .unwrap()
}

/// Kleros extra data: court id and juror count, two 32-byte words.
pub fn court_extra_data(court: u64, jurors: u64) -> Vec<u8> {
    let mut v = vec![0u8; 64];
    v[24..32].copy_from_slice(&court.to_be_bytes());
    v[56..64].copy_from_slice(&jurors.to_be_bytes());
    v
}

/// The fixture registry's governor (slot 3): the timelock in front of the Safe
/// in production; any fixed address here.
pub fn fixture_governor() -> Address {
    Address::repeat_byte(0x44)
}

/// `meta_evidence_updates` != 0 models a registry whose policy was changed
/// after deployment — verification must fail closed on it unless the profile
/// lists that version.
#[allow(dead_code)]
pub fn build_fixture_with_policy_updates(meta_evidence_updates: u64) -> Fixture {
    build_fixture_with(meta_evidence_updates, &court_extra_data(19, 3))
}

/// Solidity `bytes` storage words for `data` at `ARBITRATOR_EXTRA_DATA_SLOT`:
/// the main word, then the long-form data words if any.
pub fn extra_data_storage_words(data: &[u8]) -> Vec<(B256, U256)> {
    let main_slot = B256::from(U256::from(ARBITRATOR_EXTRA_DATA_SLOT));
    if data.len() < 32 {
        let mut w = [0u8; 32];
        w[..data.len()].copy_from_slice(data);
        w[31] = (data.len() * 2) as u8;
        return vec![(main_slot, U256::from_be_bytes(w))];
    }
    let mut out = vec![(main_slot, U256::from((data.len() * 2 + 1) as u64))];
    for (i, slot) in extra_data_slots_for_len(data.len())
        .into_iter()
        .skip(1)
        .enumerate()
    {
        let mut w = [0u8; 32];
        let chunk = &data[i * 32..((i + 1) * 32).min(data.len())];
        w[..chunk.len()].copy_from_slice(chunk);
        out.push((slot, U256::from_be_bytes(w)));
    }
    out
}

#[allow(dead_code)]
pub fn build_fixture_with(meta_evidence_updates: u64, extra_data: &[u8]) -> Fixture {
    // Four items: statuses 1 (Registered), 2 (RegistrationRequested),
    // 3 (ClearingRequested), and 0 (Absent — status slot DELETED, exclusion).
    let statuses: [u8; 4] = [1, 2, 3, 0];
    let descriptors: Vec<Descriptor> = (0..4).map(sample_descriptor).collect();
    let ids: Vec<B256> = descriptors.iter().map(|d| d.item_id()).collect();

    // Storage trie: length slot + itemList[i] + NONZERO status slots.
    let mut storage: BTreeMap<B256, Vec<u8>> = BTreeMap::new();
    let len_slot = B256::from(U256::from(ITEM_LIST_SLOT));
    let meta_slot = B256::from(U256::from(META_EVIDENCE_UPDATES_SLOT));
    storage.insert(len_slot, alloy::rlp::encode(U256::from(4u64)));
    // Arbitrator identity: slot 0 and the extra-data words (spec §6 step 3c).
    let arb_slot = B256::from(U256::from(ARBITRATOR_SLOT));
    storage.insert(
        arb_slot,
        alloy::rlp::encode(U256::from_be_bytes(fixture_arbitrator().into_word().0)),
    );
    let extra_words = extra_data_storage_words(extra_data);
    for (slot, word) in &extra_words {
        if *word != U256::ZERO {
            storage.insert(*slot, alloy::rlp::encode(*word));
        }
    }
    // Governor identity: slot 3 (spec §6 step 3d).
    let gov_slot = B256::from(U256::from(GOVERNOR_SLOT));
    storage.insert(
        gov_slot,
        alloy::rlp::encode(U256::from_be_bytes(fixture_governor().into_word().0)),
    );
    if meta_evidence_updates != 0 {
        storage.insert(
            meta_slot,
            alloy::rlp::encode(U256::from(meta_evidence_updates)),
        );
    }
    for (i, id) in ids.iter().enumerate() {
        storage.insert(
            item_list_slot(i as u64),
            alloy::rlp::encode(U256::from_be_bytes(id.0)),
        );
        if statuses[i] != 0 {
            storage.insert(
                item_status_slot(*id),
                alloy::rlp::encode(U256::from(statuses[i])),
            );
        }
    }
    // Proof targets: every slot the spec requires, INCLUDING absent ones (the
    // removed item's status slot, and — at zero updates — the policy counter).
    let mut targets: Vec<B256> = vec![len_slot, meta_slot, arb_slot, gov_slot];
    targets.extend(extra_words.iter().map(|(slot, _)| *slot));
    for (i, id) in ids.iter().enumerate() {
        targets.push(item_list_slot(i as u64));
        targets.push(item_status_slot(*id));
    }
    let (storage_root, storage_proofs) = build_trie(&storage, &targets);

    // Account trie: the registry (with the fixture codehash) + a funded EOA.
    let registry_account = TrieAccount {
        nonce: 1,
        balance: U256::from(0u64),
        storage_root,
        code_hash: registry_code_hash(),
    };
    let eoa_account = TrieAccount {
        nonce: 7,
        balance: U256::from(10u64).pow(U256::from(20u64)),
        storage_root: EMPTY_ROOT_HASH,
        code_hash: keccak_empty(),
    };
    let mut accounts = BTreeMap::new();
    accounts.insert(registry_address(), registry_account);
    accounts.insert(eoa_address(), eoa_account);
    let (state_root, account_proofs) =
        build_account_trie(&accounts, &[registry_address(), eoa_address()]);

    // Assemble the snapshot envelope with the deduplicated node store.
    let mut nodes: BTreeMap<B256, Bytes> = BTreeMap::new();
    let mut slots = Vec::new();
    for target in &targets {
        let proof = &storage_proofs[target];
        let mut path = Vec::with_capacity(proof.len());
        for node in proof {
            let h = keccak256(node);
            nodes.entry(h).or_insert_with(|| node.clone());
            path.push(h);
        }
        let value = storage
            .get(target)
            .map(|raw| alloy::rlp::decode_exact::<U256>(raw).unwrap())
            .unwrap_or(U256::ZERO);
        slots.push(SlotProof {
            slot: *target,
            value,
            path,
        });
    }
    let anchor = Anchor {
        block_number: 1000,
        block_hash: keccak256(b"fixture-block-hash"),
        state_root,
    };
    let rows: Vec<Row> = descriptors
        .iter()
        .zip(&ids)
        .zip(&statuses)
        .enumerate()
        .map(|(i, ((d, id), status))| Row {
            index: i as u64,
            item_id: *id,
            status: *status,
            descriptor: d.encode(),
        })
        .collect();
    let snapshot = Snapshot {
        version: SNAPSHOT_VERSION.into(),
        binding: Binding {
            chain_id: CHAIN_ID,
            registry: registry_address(),
        },
        anchor: anchor.clone(),
        item_count: 4,
        rows,
        proofs: Proofs {
            account_fields: AccountFields {
                nonce: registry_account.nonce,
                balance: registry_account.balance,
                storage_root,
                code_hash: registry_account.code_hash,
            },
            account: account_proofs[&registry_address()].clone(),
            nodes,
            slots,
        },
    };
    let profile = VerifierProfile {
        version: SNAPSHOT_VERSION.into(),
        chain_id: CHAIN_ID,
        registry: registry_address(),
        registry_code_hash: registry_code_hash(),
        arbitrator: fixture_arbitrator(),
        arbitrator_extra_data: Bytes::from(extra_data.to_vec()),
        governor: fixture_governor(),
        accepted_policy_updates: Vec::new(),
        anchor_block: anchor.block_number,
        anchor_block_hash: anchor.block_hash,
        anchor_state_root: anchor.state_root,
    };

    // The wrong-account artifact: the EOA's honest empty catalog under the SAME
    // state root (its empty storage trie proves slot 13 absent by exclusion —
    // with an EMPTY proof list over the empty root).
    let eoa_snapshot = Snapshot {
        version: SNAPSHOT_VERSION.into(),
        binding: Binding {
            chain_id: CHAIN_ID,
            registry: eoa_address(),
        },
        anchor,
        item_count: 0,
        rows: Vec::new(),
        proofs: Proofs {
            account_fields: AccountFields {
                nonce: eoa_account.nonce,
                balance: eoa_account.balance,
                storage_root: EMPTY_ROOT_HASH,
                code_hash: keccak_empty(),
            },
            account: account_proofs[&eoa_address()].clone(),
            nodes: BTreeMap::new(),
            slots: vec![
                SlotProof {
                    slot: B256::from(U256::from(ITEM_LIST_SLOT)),
                    value: U256::ZERO,
                    path: Vec::new(),
                },
                SlotProof {
                    slot: meta_slot,
                    value: U256::ZERO,
                    path: Vec::new(),
                },
                SlotProof {
                    slot: arb_slot,
                    value: U256::ZERO,
                    path: Vec::new(),
                },
                SlotProof {
                    slot: B256::from(U256::from(ARBITRATOR_EXTRA_DATA_SLOT)),
                    value: U256::ZERO,
                    path: Vec::new(),
                },
                SlotProof {
                    slot: gov_slot,
                    value: U256::ZERO,
                    path: Vec::new(),
                },
            ],
        },
    };

    Fixture {
        snapshot,
        profile,
        eoa_snapshot,
    }
}
