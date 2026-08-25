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
    item_list_slot, item_status_slot, AccountFields, Anchor, Binding, Proofs, Row, SlotProof,
    Snapshot, VerifierProfile, ITEM_LIST_SLOT, META_EVIDENCE_UPDATES_SLOT, SNAPSHOT_VERSION,
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
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
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
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
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
    build_fixture_with_policy_updates(0)
}

/// `meta_evidence_updates` != 0 models a registry whose policy was changed
/// after deployment — verification must fail closed on it.
#[allow(dead_code)]
pub fn build_fixture_with_policy_updates(meta_evidence_updates: u64) -> Fixture {
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
    let mut targets: Vec<B256> = vec![len_slot, meta_slot];
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
            ],
        },
    };

    Fixture {
        snapshot,
        profile,
        eoa_snapshot,
    }
}
