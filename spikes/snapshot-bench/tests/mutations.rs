//! Spot adversarial checks (brief 0002 §Goal/2): every mutated snapshot must fail
//! closed, and the profile binding must reject proofs of the WRONG account under the
//! authentic root. Deterministic and offline — runs against the committed 25-item
//! fixtures, no anvil required.

use std::path::PathBuf;

use alloy::primitives::U256;
use snapshot_bench::chain::SeedManifest;
use snapshot_bench::snapshot::{verify, Limits, Snapshot, VerifierProfile};

fn fixtures() -> (SeedManifest, Snapshot) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let manifest: SeedManifest =
        serde_json::from_slice(&std::fs::read(dir.join("manifest-25.json")).unwrap()).unwrap();
    let snapshot: Snapshot =
        serde_json::from_slice(&std::fs::read(dir.join("snapshot-25.json")).unwrap()).unwrap();
    (manifest, snapshot)
}

fn eoa_fixture() -> Snapshot {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    serde_json::from_slice(&std::fs::read(dir.join("eoa-snapshot-25.json")).unwrap()).unwrap()
}

fn expect_err(snapshot: &Snapshot, profile: &VerifierProfile, needle: &str) {
    let err = verify(snapshot, profile, &Limits::default()).expect_err("mutation must be rejected");
    let msg = format!("{err:#}");
    assert!(
        msg.contains(needle),
        "expected error containing {needle:?}, got: {msg}"
    );
}

#[test]
fn pristine_fixture_verifies_and_covers_all_statuses() {
    let (m, s) = fixtures();
    let stats = verify(&s, &m.profile(), &Limits::default()).expect("fixture must verify");
    assert_eq!(stats.items, 25);
    assert_eq!(stats.slot_proofs_checked, 51); // length + 25 list + 25 status
                                               // The fixture must exercise the whole Classic status range, INCLUDING the
                                               // zero-valued Absent slot (proven here in anvil's explicit-RLP(0x80)-leaf
                                               // form; the true exclusion form is covered by the EOA-profile test).
    for status in [0u8, 1, 2, 3] {
        assert!(
            s.rows.iter().any(|r| r.status == status),
            "fixture lacks an item with status {status}"
        );
    }
}

#[test]
fn absent_item_status_slot_proves_zero() {
    // The Absent item's status slot is zero. Anvil's fork-mode trie proves that zero
    // as an explicit RLP(0x80) LEAF (it does not delete locally zeroed slots the way
    // a real trie does), so THIS fixture covers the explicit-zero-leaf form; the
    // true exclusion form is covered end-to-end by
    // `eoa_snapshot_verifies_under_a_profile_that_pins_the_eoa`. Here we confirm the
    // row exists and its proven value is genuinely zero rather than merely unproven.
    let (_m, s) = fixtures();
    let absent = s.rows.iter().find(|r| r.status == 0).expect("absent row");
    let slot = snapshot_bench::snapshot::item_status_slot(absent.item_id);
    let sp = s
        .proofs
        .slots
        .iter()
        .find(|sp| sp.slot == slot)
        .expect("status slot proof present");
    assert_eq!(sp.value, U256::ZERO, "Absent status slot proves zero");
}

#[test]
fn rejects_wrong_trusted_root() {
    let (m, s) = fixtures();
    let mut p = m.profile();
    p.anchor_state_root.0[0] ^= 0xff;
    expect_err(&s, &p, "does not match the trusted root");
}

#[test]
fn rejects_wrong_profile_registry() {
    let (m, s) = fixtures();
    let mut p = m.profile();
    p.registry = alloy::primitives::Address::repeat_byte(0x42);
    expect_err(&s, &p, "pinned registry");
}

#[test]
fn rejects_binding_registry_tamper() {
    let (m, mut s) = fixtures();
    s.binding.registry = alloy::primitives::Address::repeat_byte(0x42);
    expect_err(&s, &m.profile(), "pinned registry");
}

#[test]
fn rejects_wrong_profile_codehash() {
    let (m, s) = fixtures();
    let mut p = m.profile();
    p.registry_code_hash.0[0] ^= 0xff;
    expect_err(&s, &p, "codeHash");
}

#[test]
fn rejects_wrong_profile_version() {
    let (m, s) = fixtures();
    let mut p = m.profile();
    p.version = "bench-9.9".into();
    expect_err(&s, &p, "version");
}

#[test]
fn rejects_wrong_profile_height() {
    let (m, s) = fixtures();
    let mut p = m.profile();
    p.anchor_block += 1;
    expect_err(&s, &p, "anchor block");
}

#[test]
fn rejects_wrong_profile_block_hash() {
    // Same height, same state root, different block hash — the spec anchors to the
    // finalized block/hash pair, so the hash is pinned and compared on its own.
    let (m, s) = fixtures();
    let mut p = m.profile();
    p.anchor_block_hash.0[0] ^= 0xff;
    expect_err(&s, &p, "anchor block hash");
}

#[test]
fn rejects_binding_block_hash_tamper() {
    let (m, mut s) = fixtures();
    s.anchor.block_hash.0[0] ^= 0xff;
    expect_err(&s, &m.profile(), "anchor block hash");
}

#[test]
fn rejects_wrong_profile_chain() {
    let (m, s) = fixtures();
    let mut p = m.profile();
    p.chain_id += 1;
    expect_err(&s, &p, "chainId");
}

#[test]
fn rejects_eoa_empty_catalog() {
    // The attack Codex named: an honest proof that a funded EOA has an empty
    // itemList slot under the AUTHENTIC anchor root, packaged as an empty catalog.
    let (m, s) = fixtures();
    let eoa = eoa_fixture();
    assert_eq!(eoa.item_count, 0);
    assert_ne!(eoa.binding.registry, s.binding.registry);
    // 1) Under the correct pinned profile: rejected on the registry pin.
    expect_err(&eoa, &m.profile(), "pinned registry");
    // 2) Even under a MISCONFIGURED profile whose registry pin is the EOA address,
    //    the proven codehash (keccak256 of empty code) cannot match the pinned
    //    runtime codehash — defense in depth.
    let mut p = m.profile();
    p.registry = eoa.binding.registry;
    expect_err(&eoa, &p, "codeHash");
}

#[test]
fn eoa_snapshot_verifies_under_a_profile_that_pins_the_eoa() {
    // Sanity for the exclusion branch, and proof that the DEFENSE is purely the
    // profile pin: the EOA snapshot is internally honest (empty storage trie ⇒ slot
    // 13 proven absent by a true exclusion proof over the empty root), so under a
    // profile that legitimately pins that EOA and the empty codehash it verifies as
    // an empty catalog. On real Gnosis state zero slots take exactly this form.
    let (m, _s) = fixtures();
    let eoa = eoa_fixture();
    let p = VerifierProfile {
        registry: eoa.binding.registry,
        registry_code_hash: alloy::primitives::KECCAK256_EMPTY,
        ..m.profile()
    };
    let stats = verify(&eoa, &p, &Limits::default()).expect("honest empty catalog");
    assert_eq!(stats.items, 0);
    assert_eq!(stats.slot_proofs_checked, 1); // just the itemList length slot
}

#[test]
fn rejects_flipped_status_claim() {
    let (m, mut s) = fixtures();
    s.rows[3].status = if s.rows[3].status == 1 { 2 } else { 1 };
    expect_err(&s, &m.profile(), "claimed status");
}

#[test]
fn rejects_absent_claimed_as_registered() {
    let (m, mut s) = fixtures();
    let i = s
        .rows
        .iter()
        .position(|r| r.status == 0)
        .expect("absent row");
    s.rows[i].status = 1; // the zero-slot proof proves 0; the claim lies
    expect_err(&s, &m.profile(), "claimed status");
}

#[test]
fn rejects_registered_claimed_as_absent() {
    let (m, mut s) = fixtures();
    let i = s
        .rows
        .iter()
        .position(|r| r.status == 1)
        .expect("registered row");
    s.rows[i].status = 0; // inclusion proof proves 1; the claim lies
    expect_err(&s, &m.profile(), "claimed status");
}

#[test]
fn rejects_tampered_descriptor() {
    let (m, mut s) = fixtures();
    let mut raw = s.rows[4].descriptor.to_vec();
    let last = raw.len() - 1;
    raw[last] ^= 0x01;
    s.rows[4].descriptor = raw.into();
    expect_err(&s, &m.profile(), "descriptor hashes to");
}

#[test]
fn rejects_noncontiguous_index() {
    let (m, mut s) = fixtures();
    s.rows[5].index = 17;
    expect_err(&s, &m.profile(), "must be contiguous");
}

#[test]
fn rejects_missing_slot_proof() {
    let (m, mut s) = fixtures();
    s.proofs.slots.pop();
    expect_err(&s, &m.profile(), "missing proof for slot");
}

#[test]
fn rejects_corrupted_node_store() {
    let (m, mut s) = fixtures();
    let key = *s.proofs.nodes.keys().next().unwrap();
    let mut node = s.proofs.nodes[&key].to_vec();
    node[0] ^= 0x01;
    s.proofs.nodes.insert(key, node.into());
    expect_err(&s, &m.profile(), "does not hash its contents");
}

#[test]
fn rejects_mutated_slot_value() {
    let (m, mut s) = fixtures();
    // Mutate a status-slot value (not the length slot, which is cross-checked first).
    let sp = s
        .proofs
        .slots
        .iter_mut()
        .next_back()
        .expect("has slot proofs");
    sp.value += U256::from(1u64);
    expect_err(&s, &m.profile(), "slot");
}

#[test]
fn rejects_truncated_rows() {
    let (m, mut s) = fixtures();
    s.rows.pop();
    expect_err(&s, &m.profile(), "");
}

#[test]
fn rejects_substituted_item_id() {
    let (m, mut s) = fixtures();
    let other = s.rows[1].item_id;
    s.rows[0].item_id = other; // descriptor no longer matches; itemList slot lies first
    expect_err(&s, &m.profile(), "");
}

#[test]
fn rejects_overclaimed_item_count() {
    let (m, mut s) = fixtures();
    s.item_count += 1;
    expect_err(&s, &m.profile(), "");
}
