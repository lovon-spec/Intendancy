//! Fully offline adversarial suite for spec §6 verification, over REAL tries
//! built in-memory (tests/common). Covers the profile bindings (incl. the EOA
//! wrong-account attack in both directions), the true-exclusion Absent slot,
//! status/descriptor/enumeration tampering, and the bounded readers.

mod common;

use alloy::primitives::{B256, U256};
use common::{
    build_fixture, build_fixture_with, build_fixture_with_policy_updates, court_extra_data,
    eoa_address, keccak_empty,
};
use intend::snapshot::{
    extra_data_slots_for_len, read_snapshot_bounded, verify, Limits, Snapshot, VerifierProfile,
    ARBITRATOR_SLOT,
};

fn expect_err(snapshot: &Snapshot, profile: &VerifierProfile, needle: &str) {
    let err = verify(snapshot, profile, &Limits::default()).expect_err("must reject");
    let msg = format!("{err:#}");
    assert!(msg.contains(needle), "expected {needle:?}, got: {msg}");
}

#[test]
fn pristine_fixture_verifies_with_all_statuses_and_exclusion() {
    let f = build_fixture();
    let stats = verify(&f.snapshot, &f.profile, &Limits::default()).expect("fixture verifies");
    assert_eq!(stats.items, 4);
    assert_eq!(stats.slot_proofs_checked, 15); // length + policy counter + arbitrator (3) + governor + 4 list + 4 status
    for status in [0u8, 1, 2, 3] {
        assert!(f.snapshot.rows.iter().any(|r| r.status == status));
    }
    // The Absent item's status slot is genuinely deleted in the fixture trie, so
    // the passing verification above exercised a TRUE exclusion proof.
    let absent = f.snapshot.rows.iter().find(|r| r.status == 0).unwrap();
    let slot = intend::snapshot::item_status_slot(absent.item_id);
    let sp = f
        .snapshot
        .proofs
        .slots
        .iter()
        .find(|sp| sp.slot == slot)
        .unwrap();
    assert_eq!(sp.value, U256::ZERO);
}

#[test]
fn rejects_wrong_profile_fields() {
    let f = build_fixture();
    let mut p = f.profile.clone();
    p.version = "9.9".into();
    expect_err(&f.snapshot, &p, "version");

    let mut p = f.profile.clone();
    p.chain_id += 1;
    expect_err(&f.snapshot, &p, "chainId");

    let mut p = f.profile.clone();
    p.registry = alloy::primitives::Address::repeat_byte(0x42);
    expect_err(&f.snapshot, &p, "pinned registry");

    let mut p = f.profile.clone();
    p.registry_code_hash.0[0] ^= 0xff;
    expect_err(&f.snapshot, &p, "codeHash");

    let mut p = f.profile.clone();
    p.anchor_block += 1;
    expect_err(&f.snapshot, &p, "anchor block");

    let mut p = f.profile.clone();
    p.anchor_block_hash.0[0] ^= 0xff;
    expect_err(&f.snapshot, &p, "anchor block hash");

    let mut p = f.profile.clone();
    p.anchor_state_root.0[0] ^= 0xff;
    expect_err(&f.snapshot, &p, "trusted root");
}

#[test]
fn rejects_eoa_empty_catalog_under_the_pinned_profile() {
    let f = build_fixture();
    // Under the correct profile: registry pin rejects.
    expect_err(&f.eoa_snapshot, &f.profile, "pinned registry");
    // Under a MISCONFIGURED profile pinning the EOA address: the proven codehash
    // (keccak256 of empty code) cannot match the pinned runtime codehash.
    let mut p = f.profile.clone();
    p.registry = eoa_address();
    expect_err(&f.eoa_snapshot, &p, "codeHash");
}

#[test]
fn eoa_snapshot_verifies_under_a_profile_that_legitimately_pins_it() {
    // Sanity: the artifact is internally honest — empty storage trie, slot 13
    // proven absent by an EMPTY exclusion proof over the empty root. The defense
    // is purely the profile pin.
    let f = build_fixture();
    let p = VerifierProfile {
        registry: eoa_address(),
        registry_code_hash: keccak_empty(),
        arbitrator: alloy::primitives::Address::ZERO,
        arbitrator_extra_data: alloy::primitives::Bytes::new(),
        governor: alloy::primitives::Address::ZERO,
        ..f.profile.clone()
    };
    let stats = verify(&f.eoa_snapshot, &p, &Limits::default()).expect("honest empty catalog");
    assert_eq!(stats.items, 0);
    assert_eq!(stats.slot_proofs_checked, 5); // length + policy counter + arbitrator + extra data + governor
}

#[test]
fn rejects_a_policy_version_the_profile_does_not_list() {
    // Owner decision 2026-09-06: the policy is mutable behind a timelock, and
    // a consumer accepts exactly the versions its signed profile lists. A
    // registry whose metaEvidenceUpdates counter moved (its trie genuinely
    // holds the nonzero value, honestly proven) fails closed under a profile
    // that lists nothing beyond the deployment version …
    let f = build_fixture_with_policy_updates(1);
    expect_err(
        &f.snapshot,
        &f.profile,
        "policy version 1 is not one the profile accepts (accepted: 0)",
    );
    // … and under one that lists a DIFFERENT version.
    let p = VerifierProfile {
        accepted_policy_updates: vec![2],
        ..f.profile.clone()
    };
    expect_err(&f.snapshot, &p, "accepted: 0, 2");
}

#[test]
fn accepts_a_listed_policy_version() {
    let f = build_fixture_with_policy_updates(1);
    let p = VerifierProfile {
        accepted_policy_updates: vec![1],
        ..f.profile.clone()
    };
    let stats = verify(&f.snapshot, &p, &Limits::default()).expect("listed version verifies");
    assert_eq!(stats.items, 4);
    // Listing a later version does not accept an earlier unlisted one, and
    // listing several accepts each of them.
    let f3 = build_fixture_with_policy_updates(3);
    let p = VerifierProfile {
        accepted_policy_updates: vec![1, 3],
        ..f3.profile.clone()
    };
    verify(&f3.snapshot, &p, &Limits::default()).expect("3 is listed");
    let f2 = build_fixture_with_policy_updates(2);
    let p2 = VerifierProfile {
        accepted_policy_updates: vec![1, 3],
        ..f2.profile.clone()
    };
    expect_err(
        &f2.snapshot,
        &p2,
        "policy version 2 is not one the profile accepts",
    );
}

#[test]
fn rejects_a_governor_the_profile_does_not_pin() {
    // Owner decision 2026-09-06: the governor (the timelock) is pinned and
    // proven at every anchor; a switch fails closed until a new profile.
    let f = build_fixture();
    let p = VerifierProfile {
        governor: alloy::primitives::Address::repeat_byte(0x55),
        ..f.profile.clone()
    };
    expect_err(&f.snapshot, &p, "governor pin violated");
}

#[test]
fn rejects_status_lies_in_both_directions() {
    let f = build_fixture();
    // Absent claimed Registered: the exclusion proof proves zero.
    let mut s = f.snapshot.clone();
    let i = s.rows.iter().position(|r| r.status == 0).unwrap();
    s.rows[i].status = 1;
    expect_err(&s, &f.profile, "claimed status");
    // Registered claimed Absent: the inclusion proof proves one.
    let mut s = f.snapshot.clone();
    let i = s.rows.iter().position(|r| r.status == 1).unwrap();
    s.rows[i].status = 0;
    expect_err(&s, &f.profile, "claimed status");
}

#[test]
fn rejects_enumeration_and_descriptor_tampering() {
    let f = build_fixture();

    let mut s = f.snapshot.clone();
    let mut raw = s.rows[1].descriptor.to_vec();
    let last = raw.len() - 1;
    raw[last] ^= 0x01;
    s.rows[1].descriptor = raw.into();
    expect_err(&s, &f.profile, "descriptor hashes to");

    let mut s = f.snapshot.clone();
    s.rows[2].index = 9;
    expect_err(&s, &f.profile, "must be contiguous");

    let mut s = f.snapshot.clone();
    s.rows.pop();
    expect_err(&s, &f.profile, "");

    let mut s = f.snapshot.clone();
    s.item_count += 1;
    expect_err(&s, &f.profile, "");

    let mut s = f.snapshot.clone();
    let swap = s.rows[1].item_id;
    s.rows[0].item_id = swap;
    expect_err(&s, &f.profile, "");

    let mut s = f.snapshot.clone();
    s.proofs.slots.pop();
    expect_err(&s, &f.profile, "missing proof for slot");

    let mut s = f.snapshot.clone();
    let key = *s.proofs.nodes.keys().next().unwrap();
    let mut node = s.proofs.nodes[&key].to_vec();
    node[0] ^= 0x01;
    s.proofs.nodes.insert(key, node.into());
    expect_err(&s, &f.profile, "does not hash its contents");
}

#[test]
fn bounded_readers_reject_oversize_and_bombs() {
    let f = build_fixture();
    let raw = serde_json::to_vec(&f.snapshot).unwrap();

    // Round-trip through the bounded reader.
    let (parsed, _) = read_snapshot_bounded(&raw, &Limits::default()).unwrap();
    verify(&parsed, &f.profile, &Limits::default()).unwrap();

    // Raw over decoded cap.
    let limits = Limits {
        max_decoded_bytes: 64,
        ..Limits::default()
    };
    assert!(read_snapshot_bounded(&raw, &limits).is_err());

    // Gzip bomb stopped at the decoded cap.
    use std::io::Write;
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(&vec![0u8; 4 * 1024 * 1024]).unwrap();
    let bomb = enc.finish().unwrap();
    let limits = Limits {
        max_decoded_bytes: 1024 * 1024,
        ..Limits::default()
    };
    let err = read_snapshot_bounded(&bomb, &limits).unwrap_err();
    assert!(
        format!("{err:#}").contains("decoded snapshot exceeds"),
        "{err:#}"
    );

    // File-boundary reader: degenerate zero cap rejects cleanly (no underflow).
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("two.json");
    std::fs::write(&path, b"{}").unwrap();
    let limits = Limits {
        max_decoded_bytes: 0,
        ..Limits::default()
    };
    let err = intend::snapshot::read_snapshot_file_bounded(&path, &limits).unwrap_err();
    assert!(format!("{err:#}").contains("decoded bound 0"), "{err:#}");
}

#[test]
fn limit_branches_reject() {
    let f = build_fixture();

    let limits = Limits {
        max_items: 2,
        ..Limits::default()
    };
    expect_err_with(&f.snapshot, &f.profile, &limits, "exceeds bound 2");

    // Row-count branch isolated from itemCount.
    let mut s = f.snapshot.clone();
    s.item_count = 1;
    let limits = Limits {
        max_items: 3,
        ..Limits::default()
    };
    let err = verify(&s, &f.profile, &limits).unwrap_err();
    assert!(format!("{err:#}").contains("row count"), "{err:#}");

    // Storage-node size branch under default limits.
    let mut s = f.snapshot.clone();
    let key = *s.proofs.nodes.keys().next().unwrap();
    s.proofs.nodes.insert(key, vec![0u8; 16 * 1024 + 1].into());
    let err = verify(&s, &f.profile, &Limits::default()).unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("exceeds 16384 bytes") && !msg.contains("account"),
        "{msg}"
    );

    // Storage-path length branch under default limits.
    let mut s = f.snapshot.clone();
    let sp = s.proofs.slots.last_mut().unwrap();
    let filler = *sp.path.first().unwrap();
    sp.path.extend(std::iter::repeat_n(filler, 70));
    let err = verify(&s, &f.profile, &Limits::default()).unwrap_err();
    assert!(format!("{err:#}").contains("proof path length"), "{err:#}");
}

fn expect_err_with(snapshot: &Snapshot, profile: &VerifierProfile, limits: &Limits, needle: &str) {
    let err = verify(snapshot, profile, limits).expect_err("must reject");
    let msg = format!("{err:#}");
    assert!(msg.contains(needle), "expected {needle:?}, got: {msg}");
}

#[test]
fn rejects_an_arbitrator_switch_and_a_court_change() {
    let f = build_fixture();
    let mut p = f.profile.clone();
    p.arbitrator = alloy::primitives::Address::repeat_byte(0x11);
    expect_err(&f.snapshot, &p, "arbitrator pin violated");

    let mut p = f.profile.clone();
    p.arbitrator_extra_data = court_extra_data(0, 3).into(); // another court, same length
    expect_err(&f.snapshot, &p, "arbitrator extra data pin violated");

    let mut p = f.profile.clone();
    p.arbitrator_extra_data = court_extra_data(19, 5).into(); // more jurors
    expect_err(&f.snapshot, &p, "arbitrator extra data pin violated");

    let mut p = f.profile.clone();
    p.arbitrator_extra_data = vec![0xaa, 0xbb, 0xcc].into(); // another length
    expect_err(&f.snapshot, &p, "arbitrator extra data pin violated");
}

#[test]
fn rejects_missing_arbitrator_proofs() {
    let f = build_fixture();
    let mut s = f.snapshot.clone();
    s.proofs
        .slots
        .retain(|sp| sp.slot != B256::from(U256::from(ARBITRATOR_SLOT)));
    expect_err(&s, &f.profile, "missing proof for slot");

    let mut s = f.snapshot.clone();
    let data_slot = extra_data_slots_for_len(64)[1];
    s.proofs.slots.retain(|sp| sp.slot != data_slot);
    expect_err(&s, &f.profile, "missing proof for slot");
}

#[test]
fn short_form_extra_data_verifies_and_is_pinned() {
    let f = build_fixture_with(0, &[0xaa, 0xbb, 0xcc]);
    let stats = verify(&f.snapshot, &f.profile, &Limits::default()).expect("short form");
    assert_eq!(stats.slot_proofs_checked, 13); // length + counter + arbitrator + main word + governor + 4 + 4

    let mut p = f.profile.clone();
    p.arbitrator_extra_data = vec![0xaa, 0xbb, 0xcd].into();
    expect_err(&f.snapshot, &p, "arbitrator extra data pin violated");
}
